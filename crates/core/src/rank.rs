//! 配信者のランク（DESIGN.md 12 章）。deadlock-api を一定間隔で取りに行き、変わったら配信する。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;

use crate::lang;
use crate::store::Store;
use crate::types::{LangCode, RankState};

const API: &str = "https://api.deadlock-api.com/v1";

#[derive(Deserialize)]
struct PlayerRank {
    badge: u32,
    rank: u32,
    subrank: u32,
    #[serde(default)]
    last_match: Option<LastMatch>,
}

#[derive(Deserialize)]
struct LastMatch {
    #[serde(default)]
    player_rank_final_flat_progress: Option<u32>,
}

#[derive(Deserialize)]
struct Tier {
    tier: u32,
    name: String,
    color: String,
    images: BTreeMap<String, Option<String>>,
}

/// ティアごとの名前（言語ごと）と色、アイコンの URL
#[derive(Default)]
struct Tiers {
    names: BTreeMap<u32, BTreeMap<LangCode, String>>,
    colors: BTreeMap<u32, String>,
    /// サブランクのないティア（オブスキュラス）のアイコン
    plain_images: BTreeMap<u32, String>,
}

pub async fn run(store: Arc<Store>, account_id: u32) {
    let languages = store.settings().glossary_languages();
    let interval = Duration::from_secs(store.settings().rank_poll_seconds);
    let mut tiers = Tiers::default();
    let mut last: Option<RankState> = None;
    let mut image: Arc<Vec<u8>> = Arc::default();
    loop {
        if tiers.names.is_empty() {
            match fetch_tiers(&languages).await {
                Ok(t) => tiers = t,
                Err(e) => tracing::warn!("rank tiers: {e:#}"),
            }
        }
        match fetch_rank(account_id).await {
            Ok(player) => {
                let state = state(&player, &tiers);
                if last.as_ref().map(|l| l.badge) != Some(state.badge) {
                    match fetch_image(&state, &tiers).await {
                        Ok(bytes) => image = Arc::new(bytes),
                        Err(e) => tracing::warn!("rank image: {e:#}"),
                    }
                }
                if last.as_ref() != Some(&state) {
                    tracing::info!("rank {} ({:?} points)", state.badge, state.points);
                    store.set_rank(state.clone(), image.clone());
                    last = Some(state);
                }
            }
            Err(e) => tracing::warn!("rank: {e:#}"),
        }
        tokio::time::sleep(interval).await;
    }
}

fn state(player: &PlayerRank, tiers: &Tiers) -> RankState {
    RankState {
        badge: player.badge,
        tier: player.rank,
        subrank: player.subrank,
        points: player
            .last_match
            .as_ref()
            .and_then(|m| m.player_rank_final_flat_progress),
        names: tiers.names.get(&player.rank).cloned().unwrap_or_default(),
        color: tiers.colors.get(&player.rank).cloned().unwrap_or_default(),
        image: format!("/assets/rank/{}.webp", player.badge),
    }
}

async fn fetch_rank(account_id: u32) -> anyhow::Result<PlayerRank> {
    Ok(crate::http()
        .get(format!("{API}/players/{account_id}/rank"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn fetch_tiers(languages: &[LangCode]) -> anyhow::Result<Tiers> {
    let mut tiers = Tiers::default();
    for code in languages {
        let api_name = lang::find(code).context("unknown language")?.api;
        let list: Vec<Tier> = crate::http()
            .get(format!("{API}/assets/ranks?language={api_name}"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        for t in list {
            tiers
                .names
                .entry(t.tier)
                .or_default()
                .insert(code.clone(), t.name);
            tiers.colors.insert(t.tier, t.color);
            if let Some(Some(url)) = t.images.get("large_webp") {
                tiers.plain_images.insert(t.tier, url.clone());
            }
        }
    }
    Ok(tiers)
}

/// サブランク付きのアイコン。サブランクのないティアはティアのアイコン
async fn fetch_image(state: &RankState, tiers: &Tiers) -> anyhow::Result<Vec<u8>> {
    let url = match (state.subrank, tiers.plain_images.get(&state.tier)) {
        (0, Some(plain)) => plain.clone(),
        _ => format!(
            "{API}/assets/ranks/{}/{}/image?format=webp",
            state.tier, state.subrank
        ),
    };
    Ok(crate::http()
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_uses_tier_names_and_final_progress() {
        let player: PlayerRank = serde_json::from_value(serde_json::json!({
            "badge": 73, "rank": 7, "subrank": 3,
            "last_match": { "match_id": 1, "start_time": 0, "player_rank_initial_display_rank": 73,
                            "player_rank_final_flat_progress": 44000 }
        }))
        .unwrap();
        let tiers = Tiers {
            names: BTreeMap::from([(7, BTreeMap::from([("ja".into(), "エミッサリー".into())]))]),
            colors: BTreeMap::from([(7, "#B47FEB".into())]),
            plain_images: BTreeMap::new(),
        };
        let s = state(&player, &tiers);
        assert_eq!(s.points, Some(44000));
        assert_eq!(s.names["ja"], "エミッサリー");
        assert_eq!(s.image, "/assets/rank/73.webp");
    }

    #[test]
    fn unranked_player_has_no_points() {
        let player: PlayerRank = serde_json::from_value(
            serde_json::json!({ "badge": 0, "rank": 0, "subrank": 0, "last_match": null }),
        )
        .unwrap();
        let s = state(&player, &Tiers::default());
        assert_eq!(s.points, None);
        assert_eq!(s.badge, 0);
    }
}
