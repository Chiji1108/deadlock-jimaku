//! ゲームの画面を読み続けて、買ったアイテムを知らせる（DESIGN.md 18.2）。
//! 読むのは、設定がオンで、見せる相手（OBS の中のオーバーレイか、読み取りの状態を出している操作パネル）がいる間だけ。

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::capture::Capture;
use super::purchases::Tracker;
use super::rates::{self, Rates};
use super::samples::{self, Sample, Samples};
use super::{BASE_HEIGHT, Frame, Read, Template, abilities, clock, heroes, items};
use crate::store::Store;
use crate::types::{Purchase, PurchasedItem, ScreenStatus};

/// 見本の画像を置く場所（データの場所の中）
const SAMPLES: &str = "screen_samples";
/// 読む間隔。1 秒に 3 回ほど
const INTERVAL: Duration = Duration::from_millis(330);
/// Deadlock のウィンドウがないときに、探し直す間隔
const SEARCH: Duration = Duration::from_secs(3);
/// 見本を取れなかったときに、取り直すまでの間
const RETRY: Duration = Duration::from_secs(30);

pub async fn run(store: Arc<Store>) {
    let mut settings = store.watch_settings();
    let mut watchers = store.screen_watchers();
    loop {
        if !wanted(&store) {
            store.set_screen_status(ScreenStatus::Off);
            tokio::select! {
                _ = settings.changed() => {}
                _ = watchers.changed() => {}
            }
            continue;
        }
        store.set_screen_status(ScreenStatus::Preparing);
        match samples::load(&store.data_dir().join(SAMPLES), None).await {
            Ok(samples) => {
                let store = store.clone();
                let runtime = tokio::runtime::Handle::current();
                let reading = tokio::task::spawn_blocking(move || read(&store, &samples, &runtime));
                if let Err(e) = reading.await {
                    tracing::error!("screen reading stopped: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("cannot load screen samples: {e:#}");
                store.set_screen_status(ScreenStatus::Failed {
                    detail: format!("{e:#}"),
                });
                tokio::time::sleep(RETRY).await;
            }
        }
    }
}

fn wanted(store: &Store) -> bool {
    *store.screen_watchers().borrow() > 0 && store.settings().purchase_cards.enabled
}

/// 読み出した画像（上の端、左下のアイテム欄、時計、下の中央のアビリティ）
type Images = [image::RgbImage; 4];
/// 自分のヒーローが決まるまで、アビリティの絵を比べる間隔（1 回に数十ミリ秒かかる）
const OWN_INTERVAL: Duration = Duration::from_secs(1);
/// 時計が読めなくなってから（ショップを開いている間など）、これより経ったら、買った時刻は分からないとする
const CLOCK_KEPT: Duration = Duration::from_secs(120);
/// ショップの外で時計が読めなくなってから、これより経ったら、時計を忘れる（画面の切り替わりの途中は待つ）
const CLOCK_GRACE: Duration = Duration::from_secs(5);

/// 画像から読んだもの
struct Reads {
    heroes: heroes::Bar,
    items: items::Slots,
    /// 試合の時計（試合が始まってからの秒）
    clock: Option<u32>,
}

/// 取り込みと、その画面の大きさに合わせた見本
struct Source {
    capture: Capture,
    scale: f32,
    heroes: Vec<Template>,
    items: Vec<Template>,
    digits: Vec<Template>,
    abilities: Vec<abilities::Shape>,
}

impl Source {
    fn open(samples: &Samples) -> anyhow::Result<Option<Self>> {
        let Some(capture) = Capture::open()? else {
            return Ok(None);
        };
        let scale = capture.size().1 as f32 / BASE_HEIGHT;
        // 読んだものは `class_name` で知らせる
        fn named(samples: &[Sample]) -> impl Iterator<Item = (&str, &image::RgbaImage)> {
            samples
                .iter()
                .map(|sample| (sample.class_name.as_str(), &sample.image))
        }
        Ok(Some(Self {
            heroes: heroes::templates(named(&samples.heroes), scale),
            items: items::templates(named(&samples.items), scale),
            digits: clock::templates(scale),
            abilities: abilities::shapes(named(&samples.abilities), scale),
            capture,
            scale,
        }))
    }

    /// 新しい画面を読む。画面が変わっていなければ None
    /// アビリティのところは、自分のヒーローを決めるときだけ読み出す（`abilities`）。読み出さなければ、空の画像
    fn read(&mut self, abilities: bool) -> anyhow::Result<Option<Images>> {
        let size = self.capture.size();
        let mut areas = vec![
            heroes::area(size, self.scale),
            items::area(size, self.scale),
            clock::area(size, self.scale),
        ];
        if abilities {
            areas.push(abilities::area(size, self.scale));
        }
        let Some(mut images) = self.capture.read(&areas)? else {
            return Ok(None);
        };
        images.resize(4, image::RgbImage::new(0, 0));
        Ok(images.try_into().ok())
    }

    /// 画像を読む。`before` は、前に読んだ画像とその答え。同じ画像のところは、照合を飛ばして前の答えを使う
    /// （上の端は後ろの景色が透けて毎回変わるが、アイテム欄は、買ったとき以外ほとんど変わらない）
    fn matched(&self, images: &Images, before: Option<(Images, Reads)>) -> Reads {
        let [top, slots, time, _] = images;
        let size = self.capture.size();
        let (mut heroes, mut items, mut clock) = (None, None, None);
        if let Some(([top_before, slots_before, time_before, _], reads)) = before {
            heroes = (top_before == *top).then_some(reads.heroes);
            items = (slots_before == *slots).then_some(reads.items);
            clock = (time_before == *time).then_some(reads.clock);
        }
        let heroes = heroes.unwrap_or_else(|| {
            let frame = Frame {
                image: top,
                origin: (0, 0),
            };
            heroes::read(&frame, &self.heroes, size.0, self.scale)
        });
        let items = items.unwrap_or_else(|| {
            let [left, upper, _, _] = items::area(size, self.scale);
            let frame = Frame {
                image: slots,
                origin: (left as i32, upper as i32),
            };
            items::read(&frame, &self.items, self.scale)
        });
        // 時計の位置は、観戦中かどうかで変わる。時計の画像が同じなら、観戦中かどうかも前と同じ
        let clock = clock.unwrap_or_else(|| {
            let [left, upper, _, _] = clock::area(size, self.scale);
            let frame = Frame {
                image: time,
                origin: (left as i32, upper as i32),
            };
            clock::read(&frame, &self.digits, size.0, self.scale, heroes.spectating)
        });
        Reads {
            heroes,
            items,
            clock,
        }
    }
}

impl Source {
    /// 下の中央のアビリティの絵から、自分のヒーロー（`class_name`）を当てる。決められなければ None
    fn own(&self, images: &Images) -> Option<String> {
        let size = self.capture.size();
        let [left, upper, _, _] = abilities::area(size, self.scale);
        let frame = Frame {
            image: &images[3],
            origin: (left as i32, upper as i32),
        };
        let reads = abilities::read(&frame, &self.abilities, size.0, self.scale);
        abilities::hero(&reads).map(str::to_owned)
    }
}

/// 自分のヒーローが出ていた枠に、別のヒーローがこの回数続けて読めたら、ヒーローを変えたと決める
const REPLACED: u32 = 3;

/// この試合の自分のヒーロー
#[derive(Default)]
struct Own {
    /// `class_name`。まだ決まっていなければ None
    hero: Option<String>,
    /// 上部バーで、自分のヒーローが出ていた枠
    slot: Option<usize>,
    /// その枠に、別のヒーローが続けて読めた回数
    replaced: u32,
}

impl Own {
    /// 上部バーの読み（枠の順。読めない枠は None）を見て、ヒーローを変えていたら忘れる。
    /// サンドボックスでは、試合の途中でヒーローを変えられる。変わるのは自分の枠だけなので、試合が変わったとは数えられない。
    /// 読めないだけの枠（調子がいいとき、死亡中）では忘れない
    fn seen(&mut self, bar: &[Option<String>]) {
        let Some(hero) = &self.hero else { return };
        if let Some(slot) = bar.iter().position(|name| name.as_ref() == Some(hero)) {
            (self.slot, self.replaced) = (Some(slot), 0);
        } else if self
            .slot
            .is_some_and(|slot| bar.get(slot).is_some_and(Option::is_some))
        {
            self.replaced += 1;
            if self.replaced >= REPLACED {
                *self = Self::default();
            }
        }
    }
}

/// 自分のヒーローの集計が届くのを、買ったと知らせる前に待つ長さ。過ぎたら、集計なしで知らせる
const RATES_WAIT: Duration = Duration::from_secs(3);

/// 買ったと決めたが、まだ知らせていないもの
struct Held {
    /// 買ったときの自分のヒーロー（`class_name`）
    hero: Option<String>,
    clock_seconds: Option<f32>,
    items: Vec<String>,
    since: Instant,
}

/// 見せる相手がいる間、画面を読み続ける。新しいフレームを待つので、専用のスレッドで呼ぶ
fn read(store: &Store, samples: &Samples, runtime: &tokio::runtime::Handle) {
    let started = Instant::now();
    let id = |samples: &[Sample], class_name: &str| {
        (samples.iter())
            .find(|sample| sample.class_name == class_name)
            .map(|sample| sample.id)
    };
    // この試合の自分のヒーロー（`class_name`）と、決めたときの試合の番号、最後に比べたとき
    let mut own = Own::default();
    let (mut own_match, mut own_tried) = (0, None::<Instant>);
    // ヒーロー（deadlock-api の id）ごとの、アイテムの集計。取れたものから入る。取れなければ、集計なしで出す
    let rates: Arc<Mutex<BTreeMap<u32, Rates>>> = Arc::default();
    let mut requested: Vec<u32> = Vec::new();
    // 買ったと決めたが、まだ知らせていないもの。自分のヒーローが決まった直後は、集計がまだ届いていない
    let mut held: VecDeque<Held> = VecDeque::new();
    let mut tracker = Tracker::default();
    let mut source: Option<Source> = None;
    // 前に読んだ画像と、その答え
    let mut last: Option<(Images, Reads)> = None;
    // 最後に読めた時計と、そのとき。ショップを開いている間は時計が隠れるので、そこから数える
    let mut clock: Option<(u32, Instant)> = None;
    while wanted(store) {
        let Some(open) = &mut source else {
            match Source::open(samples) {
                Ok(Some(opened)) => source = Some(opened),
                Ok(None) => {
                    store.set_screen_status(ScreenStatus::NoWindow);
                    std::thread::sleep(SEARCH);
                }
                Err(e) => {
                    tracing::warn!("cannot capture the game window: {e:#}");
                    store.set_screen_status(ScreenStatus::Failed {
                        detail: format!("{e:#}"),
                    });
                    std::thread::sleep(SEARCH);
                }
            }
            continue;
        };
        match open.read(own.hero.is_none()) {
            Ok(Some(images)) => {
                let reads = open.matched(&images, last.take());
                let bar: Vec<Option<String>> =
                    answers(&reads.heroes.reads, heroes::THRESHOLD).collect();
                if tracker.matches() != own_match {
                    (own, own_match) = (Own::default(), tracker.matches());
                }
                if !reads.heroes.spectating {
                    own.seen(&bar);
                }
                // 観戦中は、下の中央も相手のアビリティになる
                let due = own_tried.is_none_or(|at| at.elapsed() >= OWN_INTERVAL);
                // 読み出していなければ（前の回に決まっていた）、次の回に比べる
                let read_out = images[3].width() > 0;
                if own.hero.is_none() && due && read_out && !reads.heroes.spectating {
                    own_tried = Some(Instant::now());
                    own.hero = open.own(&images);
                    if let Some(hero) =
                        (own.hero.as_deref()).and_then(|own| id(&samples.heroes, own))
                        && !requested.contains(&hero)
                    {
                        requested.push(hero);
                        let rates = rates.clone();
                        runtime.spawn(async move {
                            match rates::fetch(hero).await {
                                Ok(fetched) => {
                                    rates.lock().unwrap().insert(hero, fetched);
                                }
                                Err(e) => {
                                    // 取れなかったら、集計なしで出す（買うたびに待たせない）
                                    tracing::warn!("cannot fetch item rates: {e:#}");
                                    rates.lock().unwrap().insert(hero, Rates::default());
                                }
                            }
                        });
                    }
                }
                let (width, height) = open.capture.size();
                store.set_screen_status(ScreenStatus::Reading {
                    width,
                    height,
                    heroes: bar,
                    items: answers(&reads.items.reads, items::THRESHOLD)
                        .flatten()
                        .collect(),
                    shop: reads.items.shop,
                    spectating: reads.heroes.spectating,
                    own: own.hero.clone(),
                    clock: reads.clock,
                });
                if let Some(seconds) = reads.clock {
                    clock = Some((seconds, Instant::now()));
                } else if !reads.items.shop
                    && clock.is_some_and(|(_, at)| at.elapsed() >= CLOCK_GRACE)
                {
                    // ショップの外で読めないのは、時計のない画面（メニュー、サンドボックス）。前の試合の時計から数えない
                    clock = None;
                }
                let bought = tracker.observe(started.elapsed(), &reads.heroes, &reads.items);
                if !bought.is_empty() {
                    held.push_back(Held {
                        hero: own.hero.clone(),
                        clock_seconds: clock
                            .filter(|(_, at)| at.elapsed() < CLOCK_KEPT)
                            .map(|(seconds, at)| seconds as f32 + at.elapsed().as_secs_f32()),
                        items: bought,
                        since: Instant::now(),
                    });
                }
                last = Some((images, reads));
            }
            // 画面が変わっていないか、最小化している
            Ok(None) => {}
            // ウィンドウが閉じたか、大きさが変わった。開き直す
            Err(e) => {
                tracing::info!("reopening the capture: {e:#}");
                source = None;
            }
        }
        // 集計が届いたものから知らせる
        while let Some(next) = held.front() {
            let fetched = rates.lock().unwrap();
            let hero = (next.hero.as_deref()).and_then(|hero| id(&samples.heroes, hero));
            let rates = hero.and_then(|hero| fetched.get(&hero));
            if hero.is_some() && rates.is_none() && next.since.elapsed() < RATES_WAIT {
                break;
            }
            let items = (next.items.iter())
                .map(|item| PurchasedItem {
                    rate: rates
                        .zip(id(&samples.items, item))
                        .and_then(|(rates, id)| rates.of(id)),
                    item: item.clone(),
                })
                .collect();
            store.purchased(Purchase {
                hero: next.hero.clone(),
                clock_seconds: next.clock_seconds,
                items,
            });
            drop(fetched);
            held.pop_front();
        }
        std::thread::sleep(INTERVAL);
    }
}

fn answers(reads: &[Read], threshold: f32) -> impl Iterator<Item = Option<String>> {
    reads
        .iter()
        .map(move |read| read.answer(threshold).map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(names: &[Option<&str>]) -> Vec<Option<String>> {
        names.iter().map(|name| name.map(str::to_owned)).collect()
    }

    #[test]
    fn another_hero_in_the_own_slot_means_the_hero_was_changed() {
        let mut own = Own {
            hero: Some("hero_chessmaster".to_owned()),
            ..Own::default()
        };
        let solomon = bar(&[Some("hero_inferno"), Some("hero_chessmaster"), None]);
        own.seen(&solomon);
        // 調子がいいときや死亡中は、自分の枠が読めないだけ
        let unreadable = bar(&[Some("hero_inferno"), None, None]);
        for _ in 0..10 {
            own.seen(&unreadable);
        }
        assert_eq!(own.hero.as_deref(), Some("hero_chessmaster"));
        // サンドボックスでヒーローを変えた。1 回だけなら読み間違いかもしれない
        let changed = bar(&[Some("hero_inferno"), Some("hero_vampirebat"), None]);
        own.seen(&changed);
        own.seen(&solomon);
        own.seen(&changed);
        own.seen(&changed);
        assert_eq!(own.hero.as_deref(), Some("hero_chessmaster"));
        own.seen(&changed);
        assert_eq!(own.hero, None);
    }
}
