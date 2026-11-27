//! 読んだ持ち物の移り変わりから、買ったアイテムを出す（DESIGN.md 18.3）。画面も通信も触らない。
//!
//! 持ち物は名前の集まりで持つ（買うと並びが入れ替わるので、枠では比べない）。
//! 空の枠は背景が透けていて、読めないだけの枠（クールダウンで暗い、ほかの表示に隠れた）と見分けにくいので、
//! 読めなくなっただけでは「なくなった」と決めない。試合の画面が見えているのに、どの枠にも薄くすら出ていないのが
//! しばらく続いたときだけ、売った（または作り替えた）と決める。そうしないと、買い直したときに出ない。

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use super::{heroes, items};

/// 続けてこの回数だけ読めたら、持っていると決める（画面の切り替わりの途中を読んだ間違いを出さない）
const CONFIRM: u32 = 4;
/// 読み始めてから（試合が変わってから）この回数は、読めた持ち物を基準にするだけで、買ったとは出さない
const BASELINE: u32 = 3;
/// これだけの間、上部バーもアイテム欄も読めなければ、試合が終わったことにする
const GAP: Duration = Duration::from_secs(30);
/// 上部バーのこの数の枠が、前と違うヒーローに変わっていたら、次の試合が始まったことにする
const ROSTER_CHANGES: usize = 2;
/// ショップを開かずに、一度にこれより多く増えたら、買ったのではなく、欄が隠れていただけ（スコアボードを
/// 開いたまま読み始めた、など）。ショートカットで買うときは、1 つずつ増える
const AT_ONCE: usize = 2;
/// ショップを閉じてから、この回数は、まとめて増えても買ったものとして出す
const AFTER_SHOP: u32 = 6;
/// クールダウンで暗い枠は、近さは足りないが、ほかの見本とは離れている（0.67〜0.77 と、次点 0.30〜0.45）。
/// 空の枠は、いちばん近い見本と次点が近い（0.73 と 0.64）。基準を作るときだけ、暗い枠も持ち物に数える
const FAINT: (f32, f32) = (0.6, 0.2);
/// 持っているアイテムが、どの枠のいちばん近い見本にもなっていない（これより近くない）のが、
/// この回数続いたら、売ったと決める（5 秒ほど）。クールダウンで暗い枠は、近さは落ちても、いちばん近い見本はそのアイテムのまま
const GONE: (f32, u32) = (0.45, 15);

#[derive(Default)]
pub struct Tracker {
    /// 持っていると決めたアイテム（基準にしたものと、買ったと出したもの）
    owned: BTreeSet<String>,
    /// まだ持っていると決めていないアイテムが、続けて読めた回数
    seen: BTreeMap<String, u32>,
    /// 基準を作る残りの回数
    baseline: Option<u32>,
    shop: bool,
    after_shop: u32,
    /// 上部バーの枠ごとの、この試合で初めに読めたヒーロー
    roster: Vec<Option<String>>,
    /// 初めに読めたのと違うヒーローが読めた枠。同じ画面で揃って読めるとは限らないので、覚えておく
    moved: BTreeSet<usize>,
    readable_at: Option<Duration>,
    /// 持っているアイテムが、どの枠にも出ていない回数
    gone: BTreeMap<String, u32>,
    /// 試合が変わったと決めた回数
    matches: u32,
}

impl Tracker {
    /// 試合が変わったと決めた回数。変わったら、試合ごとに覚えたもの（自分のヒーローなど）を捨てる
    pub fn matches(&self) -> u32 {
        self.matches
    }

    /// 1 回読んだ結果を渡す。買ったと決めたアイテムを、枠の順に返す。`at` は読んだ時刻（どこから測ってもよい）
    pub fn observe(
        &mut self,
        at: Duration,
        bar: &heroes::Bar,
        items: &items::Slots,
    ) -> Vec<String> {
        if bar.spectating {
            // 左下に出ているのは、観戦している相手の持ち物
            self.readable_at = Some(at);
            self.seen.clear();
            self.gone.clear();
            return Vec::new();
        }
        let heroes: Vec<Option<&str>> = bar
            .reads
            .iter()
            .map(|read| read.answer(heroes::THRESHOLD))
            .collect();
        // 枠は 1 つずつ読むので、2 つの枠が同じアイテムと読めることがある。1 回の読み取りでは 1 回に数える
        let mut known: Vec<&str> = Vec::new();
        for name in (items.reads.iter()).filter_map(|read| read.answer(items::THRESHOLD)) {
            if !known.contains(&name) {
                known.push(name);
            }
        }

        if heroes.iter().all(Option::is_none) && known.is_empty() {
            // 試合の外か、ほかの画面に隠れている。ショップを開いている間は、上部バーがショップに、
            // アイテム欄が説明のカードに隠れるので、長く読めなくても試合は変わっていない
            if !self.shop
                && self
                    .readable_at
                    .is_some_and(|last| at.saturating_sub(last) >= GAP)
            {
                *self = Self {
                    matches: self.matches + 1,
                    ..Self::default()
                };
            }
            self.seen.clear();
            return Vec::new();
        }
        self.readable_at = Some(at);

        self.roster.resize(heroes.len(), None);
        for (slot, (before, now)) in self.roster.iter().zip(&heroes).enumerate() {
            match (before, now) {
                (Some(before), Some(now)) if before != now => self.moved.insert(slot),
                // 読み間違いだった
                (Some(_), Some(_)) => self.moved.remove(&slot),
                _ => false,
            };
        }
        if self.moved.len() >= ROSTER_CHANGES {
            *self = Self {
                roster: vec![None; heroes.len()],
                readable_at: Some(at),
                matches: self.matches + 1,
                ..Self::default()
            };
        }
        for (slot, hero) in self.roster.iter_mut().zip(&heroes) {
            if let (None, Some(hero)) = (&slot, hero) {
                *slot = Some((*hero).to_owned());
            }
        }

        if items.shop != self.shop {
            // 欄の位置が変わる途中を読んでいるかもしれないので、数え直す
            self.seen.clear();
            self.after_shop = if items.shop { 0 } else { AFTER_SHOP };
            self.shop = items.shop;
        }

        let hidden = heroes.iter().all(Option::is_none);
        let baseline = self.baseline.get_or_insert(BASELINE);
        if *baseline > 0 {
            if hidden || items.shop {
                // ショップの中は、説明のカードに隠れた枠が読めない。基準は、試合の画面が見えているときに作る
                return Vec::new();
            }
            *baseline -= 1;
            let faint = items
                .reads
                .iter()
                .filter(|read| read.score >= FAINT.0 && read.score - read.second >= FAINT.1);
            self.owned.extend(faint.map(|read| read.name.clone()));
            self.owned
                .extend(known.iter().map(|name| (*name).to_owned()));
            return Vec::new();
        }

        if hidden && !items.shop {
            // 上部バーが隠れている（観戦に入る途中など）。ショップの中は、閉じたあとに数える
            self.seen.clear();
            self.gone.clear();
            return Vec::new();
        }
        self.seen.retain(|name, _| known.contains(&name.as_str()));
        for name in &known {
            if !self.owned.contains(*name) {
                *self.seen.entry((*name).to_owned()).or_default() += 1;
            }
        }
        if self.shop {
            // 開いている間は出さない（買い直しがよくある）。閉じたときに、開く前から増えた分が出る。
            // 説明のカードに隠れた枠は読めないので、なくなったかも数えない
            self.gone.clear();
            return Vec::new();
        }
        // 試合の画面が見えているのに、どの枠にも出ていないアイテムは、売ったか作り替えた。
        // 覚えたままにすると、買い直したときに出ない
        let shown =
            |name: &str| (items.reads.iter()).any(|read| read.name == name && read.score >= GONE.0);
        self.gone.retain(|name, _| !shown(name));
        for name in self.owned.iter().filter(|name| !shown(name)) {
            *self.gone.entry(name.clone()).or_default() += 1;
        }
        self.gone.retain(|name, count| {
            let sold = *count >= GONE.1;
            if sold {
                self.owned.remove(name);
            }
            !sold
        });
        let from_shop = self.after_shop > 0;
        self.after_shop = self.after_shop.saturating_sub(1);
        let bought: Vec<String> = known
            .iter()
            .filter(|name| self.seen.get(**name).is_some_and(|count| *count >= CONFIRM))
            .map(|name| (*name).to_owned())
            .collect();
        for name in &bought {
            self.seen.remove(name);
            self.owned.insert(name.clone());
        }
        if bought.len() > AT_ONCE && !from_shop {
            return Vec::new();
        }
        bought
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::Read;

    fn read(name: &str, score: f32, second: f32) -> Read {
        Read {
            name: name.to_owned(),
            score,
            second,
        }
    }

    /// 空の枠（背景が透けて、どの見本にも同じくらい近い）
    fn empty() -> Read {
        read("Active Reload", 0.7, 0.63)
    }

    /// クールダウンで暗い枠
    fn dark(name: &str) -> Read {
        read(name, 0.7, 0.35)
    }

    fn heroes(names: &[&str]) -> heroes::Bar {
        let mut reads: Vec<Read> = names.iter().map(|name| read(name, 0.95, 0.4)).collect();
        reads.resize(12, read("Haze", 0.5, 0.45));
        heroes::Bar {
            reads,
            spectating: false,
        }
    }

    fn slots(reads: Vec<Read>, shop: bool) -> items::Slots {
        let mut reads = reads;
        reads.resize(12, empty());
        items::Slots { reads, shop }
    }

    fn held(names: &[&str]) -> items::Slots {
        slots(
            names.iter().map(|name| read(name, 0.97, 0.5)).collect(),
            false,
        )
    }

    fn in_shop(names: &[&str]) -> items::Slots {
        slots(
            names.iter().map(|name| read(name, 0.97, 0.5)).collect(),
            true,
        )
    }

    /// 1 秒に 3 回読む
    struct Game {
        tracker: Tracker,
        ticks: u32,
        team: heroes::Bar,
    }

    impl Game {
        fn new() -> Self {
            Self {
                tracker: Tracker::default(),
                ticks: 0,
                team: heroes(&["McGinnis", "Kelvin", "The Doorman"]),
            }
        }

        fn see(&mut self, items: &items::Slots) -> Vec<String> {
            self.ticks += 1;
            let at = Duration::from_millis(333 * self.ticks as u64);
            self.tracker.observe(at, &self.team, items)
        }

        /// 同じ画面を何回か読んで、出たものを集める
        fn watch(&mut self, items: &items::Slots, times: u32) -> Vec<String> {
            (0..times).flat_map(|_| self.see(items)).collect()
        }

        /// 上部バーもアイテム欄も読めない画面が続く
        fn nothing(&mut self, seconds: u64) {
            self.ticks += (seconds * 3) as u32;
            let at = Duration::from_millis(333 * self.ticks as u64);
            let hidden = slots(Vec::new(), false);
            let nobody = heroes(&[]);
            assert!(self.tracker.observe(at, &nobody, &hidden).is_empty());
        }

        /// 試合の途中から読み始めて、いま持っているものを基準にする
        fn started_with(names: &[&str]) -> Self {
            let mut game = Self::new();
            assert!(game.watch(&held(names), 5).is_empty());
            game
        }
    }

    #[test]
    fn what_is_held_at_the_start_is_not_a_purchase() {
        let mut game = Game::new();
        let start = slots(
            vec![read("Headhunter", 0.98, 0.6), dark("Tankbuster")],
            false,
        );
        assert!(game.watch(&start, 5).is_empty());
        // クールダウンが明けて、はっきり読めるようになっても、買ったことにしない
        assert!(
            game.watch(&held(&["Headhunter", "Tankbuster"]), 5)
                .is_empty()
        );
    }

    #[test]
    fn a_new_item_is_bought_once_it_is_read_several_times_in_a_row() {
        let mut game = Game::started_with(&["Headhunter"]);
        let after = held(&["Extra Regen", "Headhunter"]);
        assert!(game.watch(&after, CONFIRM - 1).is_empty());
        assert_eq!(game.see(&after), ["Extra Regen"]);
        assert!(game.watch(&after, 5).is_empty());
    }

    #[test]
    fn the_first_item_of_a_match_is_a_purchase() {
        let mut game = Game::new();
        assert!(game.watch(&held(&[]), 9).is_empty());
        assert_eq!(
            game.watch(&held(&["Extra Regen"]), CONFIRM),
            ["Extra Regen"]
        );
    }

    #[test]
    fn something_read_only_once_is_not_a_purchase() {
        let mut game = Game::started_with(&["Headhunter"]);
        assert!(game.see(&held(&["Headhunter", "Berserker"])).is_empty());
        assert!(game.watch(&held(&["Headhunter"]), 3).is_empty());
        assert!(game.see(&held(&["Headhunter", "Berserker"])).is_empty());
    }

    #[test]
    fn a_new_order_and_a_cooldown_are_not_purchases() {
        let mut game = Game::started_with(&["Headhunter", "Berserker", "Fortitude"]);
        assert!(
            game.watch(&held(&["Fortitude", "Headhunter", "Berserker"]), 3)
                .is_empty()
        );
        let cooling = slots(
            vec![
                read("Fortitude", 0.97, 0.5),
                dark("Headhunter"),
                read("Berserker", 0.97, 0.5),
            ],
            false,
        );
        assert!(game.watch(&cooling, 6).is_empty());
        assert!(
            game.watch(&held(&["Fortitude", "Headhunter", "Berserker"]), 3)
                .is_empty()
        );
    }

    #[test]
    fn in_the_shop_only_what_is_left_at_closing_is_a_purchase() {
        let mut game = Game::started_with(&["Headhunter"]);
        assert!(game.watch(&in_shop(&["Headhunter"]), 3).is_empty());
        // 買って、間違えたので売って、別のものを買う
        assert!(
            game.watch(&in_shop(&["Headhunter", "Berserker"]), 4)
                .is_empty()
        );
        assert!(game.watch(&in_shop(&["Headhunter"]), CONFIRM).is_empty());
        assert!(
            game.watch(&in_shop(&["Headhunter", "Fortitude"]), 4)
                .is_empty()
        );
        // 説明のカードに欄が隠れる
        assert!(game.watch(&slots(Vec::new(), true), CONFIRM).is_empty());
        assert_eq!(
            game.watch(&held(&["Fortitude", "Headhunter"]), CONFIRM),
            ["Fortitude"]
        );
    }

    #[test]
    fn what_was_sold_is_a_purchase_when_bought_again() {
        let mut game = Game::started_with(&["Headhunter", "Berserker"]);
        // 売った。どの枠にも出ていないのが続く
        assert!(game.watch(&held(&["Headhunter"]), GONE.1).is_empty());
        assert_eq!(
            game.watch(&held(&["Headhunter", "Berserker"]), CONFIRM),
            ["Berserker"]
        );
        // 全部売って、買い直す
        assert!(game.watch(&held(&[]), GONE.1).is_empty());
        assert_eq!(game.watch(&held(&["Berserker"]), CONFIRM), ["Berserker"]);
    }

    #[test]
    fn what_is_only_unreadable_was_not_sold() {
        let mut game = Game::started_with(&["Headhunter", "Berserker"]);
        // クールダウンで、長く暗い。近さは落ちても、いちばん近い見本はそのアイテムのまま
        let dim = slots(
            vec![read("Headhunter", 0.97, 0.5), read("Berserker", 0.5, 0.46)],
            false,
        );
        assert!(game.watch(&dim, 200).is_empty());
        assert!(
            game.watch(&held(&["Headhunter", "Berserker"]), 9)
                .is_empty()
        );
        // ショップの説明のカードや、観戦中、上部バーが隠れている間は、数えない
        assert!(game.watch(&in_shop(&["Headhunter"]), 60).is_empty());
        game.team = heroes(&[]);
        assert!(game.watch(&held(&["Headhunter"]), 60).is_empty());
        game.team = heroes::Bar {
            spectating: true,
            ..heroes(&["McGinnis", "Kelvin", "The Doorman"])
        };
        assert!(game.watch(&held(&["Fortitude"]), 60).is_empty());
        game.team = heroes(&["McGinnis", "Kelvin", "The Doorman"]);
        // 少しの間だけ読めなくても、売ったとは決めない
        assert!(game.watch(&held(&["Headhunter"]), GONE.1 - 1).is_empty());
        assert!(
            game.watch(&held(&["Headhunter", "Berserker"]), 9)
                .is_empty()
        );
    }

    #[test]
    fn several_items_from_one_visit_to_the_shop_come_together() {
        let mut game = Game::started_with(&[]);
        let bought = ["Extra Regen", "Headhunter", "Berserker", "Fortitude"];
        assert!(game.watch(&in_shop(&bought), 6).is_empty());
        let after = held(&bought);
        assert!(game.watch(&after, CONFIRM - 1).is_empty());
        assert_eq!(game.see(&after), bought);
    }

    #[test]
    fn many_items_appearing_at_once_outside_the_shop_were_only_hidden() {
        // スコアボードを開いたまま読み始めて、閉じた
        let mut game = Game::new();
        assert!(game.watch(&held(&[]), 6).is_empty());
        let shown = held(&["Extra Regen", "Headhunter", "Berserker"]);
        assert!(game.watch(&shown, 4).is_empty());
        assert_eq!(
            game.watch(
                &held(&["Fortitude", "Extra Regen", "Headhunter", "Berserker"]),
                CONFIRM
            ),
            ["Fortitude"]
        );
    }

    #[test]
    fn a_short_time_with_nothing_readable_keeps_what_is_held() {
        let mut game = Game::started_with(&["Headhunter"]);
        game.nothing(10);
        assert!(game.watch(&held(&["Headhunter"]), 4).is_empty());
    }

    #[test]
    fn the_next_match_starts_over() {
        let mut game = Game::started_with(&["Headhunter"]);
        game.nothing(40);
        assert!(game.watch(&held(&[]), 5).is_empty());
        assert_eq!(game.watch(&held(&["Headhunter"]), CONFIRM), ["Headhunter"]);
    }

    #[test]
    fn items_of_a_spectated_player_are_not_purchases() {
        let mut game = Game::started_with(&["Headhunter"]);
        let mine = heroes(&["McGinnis", "Kelvin", "The Doorman"]);
        // 死んだ。バーが下がる途中は、上部バーが読めないのに、左下はもう相手の持ち物
        game.team = heroes(&[]);
        assert!(game.watch(&held(&["Berserker"]), 6).is_empty());
        game.team = heroes::Bar {
            spectating: true,
            ..heroes(&["McGinnis", "Kelvin", "The Doorman"])
        };
        assert!(game.watch(&held(&["Berserker"]), 90).is_empty());
        // リスポーンした。相手の持ち物は覚えていないので、あとで自分が買えば出る
        game.team = mine;
        assert!(game.watch(&held(&["Headhunter"]), 6).is_empty());
        assert_eq!(
            game.watch(&held(&["Headhunter", "Berserker"]), CONFIRM),
            ["Berserker"]
        );
    }

    #[test]
    fn a_long_visit_to_the_shop_does_not_forget_what_is_held() {
        let mut game = Game::started_with(&["Headhunter", "Berserker"]);
        // ショップを開くと、上部バーはショップに隠れる
        game.team = heroes(&[]);
        assert!(
            game.watch(&in_shop(&["Headhunter", "Berserker"]), 3)
                .is_empty()
        );
        // 説明のカードがアイテム欄を隠したまま、長く見ている
        assert!(game.watch(&slots(Vec::new(), false), 120).is_empty());
        // 1 つ買う。カードに隠れて、持っていたものが 1 つ読めない
        assert!(
            game.watch(&in_shop(&["Berserker", "Fortitude"]), 6)
                .is_empty()
        );
        assert_eq!(game.tracker.matches(), 0);
        // 閉じた。出るのは、買ったものだけ
        game.team = heroes(&["McGinnis", "Kelvin", "The Doorman"]);
        assert_eq!(
            game.watch(&held(&["Headhunter", "Berserker", "Fortitude"]), CONFIRM),
            ["Fortitude"]
        );
    }

    #[test]
    fn the_baseline_waits_until_the_shop_is_closed() {
        // ショップを開いたまま読み始めた。説明のカードに隠れて、持っているものが 1 つ読めない
        let mut game = Game::new();
        game.team = heroes(&[]);
        assert!(game.watch(&in_shop(&["Berserker"]), 9).is_empty());
        game.team = heroes(&["McGinnis", "Kelvin", "The Doorman"]);
        assert!(
            game.watch(&held(&["Headhunter", "Berserker"]), 9)
                .is_empty()
        );
    }

    #[test]
    fn the_same_item_in_two_slots_of_one_reading_counts_once() {
        let mut game = Game::started_with(&["Headhunter"]);
        let twice = held(&["Berserker", "Headhunter", "Berserker"]);
        assert!(game.watch(&twice, CONFIRM - 1).is_empty());
        assert_eq!(game.see(&twice), ["Berserker"]);
    }

    #[test]
    fn the_heroes_of_the_next_match_may_become_readable_one_at_a_time() {
        let mut game = Game::started_with(&["Headhunter"]);
        // 1 枠だけ変わって読めても、読み間違いかもしれない
        game.team = heroes(&["Rem", "Kelvin", "The Doorman"]);
        assert_eq!(
            game.watch(&held(&["Headhunter", "Berserker"]), CONFIRM),
            ["Berserker"]
        );
        // 読み間違いだったなら、数えない
        game.team = heroes(&["McGinnis", "Kelvin", "The Doorman"]);
        game.see(&held(&["Headhunter", "Berserker"]));
        game.team = heroes(&["McGinnis", "Yamato", "The Doorman"]);
        assert!(
            game.watch(&held(&["Headhunter", "Berserker"]), CONFIRM)
                .is_empty()
        );

        assert_eq!(game.tracker.matches(), 0);

        // 別の画面で 1 枠ずつ変わって読めたら、次の試合
        let mut game = Game::started_with(&["Headhunter"]);
        let mut first = heroes(&["Rem"]);
        first.reads[1] = read("Haze", 0.5, 0.45);
        game.team = first;
        game.see(&held(&[]));
        game.team = heroes(&["Rem", "Yamato", "The Doorman"]);
        assert!(game.watch(&held(&[]), 5).is_empty());
        assert_eq!(game.tracker.matches(), 1);
        assert_eq!(game.watch(&held(&["Headhunter"]), CONFIRM), ["Headhunter"]);
    }

    #[test]
    fn other_heroes_in_the_top_bar_mean_the_next_match() {
        let mut game = Game::started_with(&["Headhunter"]);
        game.team = heroes(&["Rem", "Yamato", "The Doorman"]);
        assert!(game.watch(&held(&[]), 5).is_empty());
        assert_eq!(game.watch(&held(&["Headhunter"]), CONFIRM), ["Headhunter"]);
    }
}
