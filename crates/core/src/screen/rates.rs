//! あるヒーローで、アイテムがどのくらい買われているか（DESIGN.md 18.4）。deadlock-api の集計から取る。

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;

const API: &str = "https://api.deadlock-api.com/v1/analytics";
/// 集計する期間。API は、指定しなくても最近の試合だけを数えるが、期間を決めておく
const PERIOD: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    /// そのヒーローの試合のうち、このアイテムを買った試合の割合（0〜1）
    pub share: f32,
    /// 買った時刻の平均（試合が始まってからの秒）
    pub bought_at_seconds: f32,
}

/// ヒーロー 1 人分の、アイテムごとの集計
#[derive(Debug, Default)]
pub struct Rates {
    items: BTreeMap<u32, Rate>,
}

impl Rates {
    /// アイテム（deadlock-api の id）の集計。そのヒーローで買われた記録がなければ None
    pub fn of(&self, item: u32) -> Option<Rate> {
        self.items.get(&item).copied()
    }
}

#[derive(Deserialize)]
struct HeroStats {
    hero_id: u32,
    matches: u64,
}

#[derive(Deserialize)]
struct ItemStats {
    item_id: u32,
    matches: u64,
    avg_buy_time_s: f32,
}

/// ヒーロー（deadlock-api の id）の集計を取る
pub async fn fetch(hero: u32) -> anyhow::Result<Rates> {
    // 1 日のうちは同じ URL にして、CDN の持っている分を使う
    let day = 24 * 60 * 60;
    let since = (crate::now_ms() / 1000 / day * day).saturating_sub(PERIOD.as_secs());
    let get = |url: String| async move {
        let response = crate::http().get(&url).send().await?.error_for_status()?;
        anyhow::Ok(response.bytes().await?)
    };
    let (heroes, items) = tokio::try_join!(
        get(format!("{API}/hero-stats?min_unix_timestamp={since}")),
        get(format!(
            "{API}/item-stats?hero_id={hero}&min_unix_timestamp={since}"
        )),
    )?;
    rates(hero, &heroes, &items)
}

fn rates(hero: u32, heroes: &[u8], items: &[u8]) -> anyhow::Result<Rates> {
    let heroes: Vec<HeroStats> = serde_json::from_slice(heroes).context("hero-stats")?;
    let items: Vec<ItemStats> = serde_json::from_slice(items).context("item-stats")?;
    let played = heroes
        .iter()
        .find(|stats| stats.hero_id == hero)
        .map(|stats| stats.matches)
        .filter(|matches| *matches > 0)
        .context("no matches for the hero")?;
    let items = items.into_iter().map(|stats| {
        let rate = Rate {
            share: stats.matches as f32 / played as f32,
            bought_at_seconds: stats.avg_buy_time_s,
        };
        (stats.item_id, rate)
    });
    Ok(Rates {
        items: items.collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEROES: &[u8] = br#"[{"hero_id": 13, "matches": 500, "wins": 250},
        {"hero_id": 69, "matches": 1000, "wins": 480}]"#;
    const ITEMS: &[u8] = br#"[{"item_id": 7, "matches": 880, "wins": 400, "avg_buy_time_s": 348.0},
        {"item_id": 9, "matches": 1, "wins": 1, "avg_buy_time_s": 1680.5}]"#;

    #[test]
    fn the_share_is_the_matches_with_the_item_out_of_the_matches_of_the_hero() {
        let rates = rates(69, HEROES, ITEMS).unwrap();
        let common = rates.of(7).unwrap();
        assert_eq!(common.share, 0.88);
        assert_eq!(common.bought_at_seconds, 348.0);
        assert_eq!(rates.of(9).unwrap().share, 0.001);
        assert_eq!(rates.of(8), None);
    }

    #[test]
    fn a_hero_without_matches_has_no_rates() {
        assert!(rates(1, HEROES, ITEMS).is_err());
        assert!(rates(69, br#"[{"hero_id": 69, "matches": 0}]"#, ITEMS).is_err());
    }
}
