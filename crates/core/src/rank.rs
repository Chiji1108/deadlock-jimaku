//! 配信者のランク（DESIGN.md 12 章）。deadlock-api を一定間隔で取りに行き、変わったら配信する。

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use bytes::Bytes;
use serde::Deserialize;

use crate::lang;
use crate::store::Store;
use crate::types::{LangCode, RankProgress, RankState};

const API: &str = "https://api.deadlock-api.com/v1";

/// サブランク 1 つ分のポイント。6 番目だけ 2 倍（1 ティア = 1000 × 5 + 2000）
const SUBRANK_POINTS: u32 = 1000;
const TIER_POINTS: u32 = SUBRANK_POINTS * 7;
/// 最上位。サブランクはポイントではなく上位何 % かで決まるのでゲージを出さない
const ETERNUS: u32 = 11;

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

/// ティアごとの名前（言語ごと）と色、サブランクのないティアのアイコン
#[derive(Default)]
struct Tiers {
    names: BTreeMap<u32, BTreeMap<LangCode, String>>,
    colors: BTreeMap<u32, String>,
    plain_images: BTreeMap<u32, String>,
    /// 名前を取れた言語。取れなかった言語は次の回に取り直す
    loaded: Vec<LangCode>,
}

/// 設定にアカウントがあれば、ランクを取りに行き続ける
pub async fn run_configured(store: Arc<Store>) {
    if let Some(account_id) = store.settings().rank_account {
        run(store, account_id).await;
    }
}

async fn run(store: Arc<Store>, account_id: u32) {
    let languages = store.settings().glossary_languages();
    let interval = Duration::from_secs(store.settings().rank_poll_seconds);
    let mut tiers = Tiers::default();
    let mut last: Option<RankState> = None;
    // アイコンを持っているバッジ。取れなかったら次の回に取り直す
    let mut image_badge: Option<u32> = None;
    loop {
        load_tiers(&mut tiers, &languages).await;
        match fetch_rank(account_id).await {
            Ok(player) => {
                if image_badge != Some(player.badge) {
                    match fetch_image(&player, &tiers).await {
                        Ok(bytes) => {
                            store.set_rank_image(player.badge, bytes);
                            image_badge = Some(player.badge);
                        }
                        Err(e) => tracing::warn!("rank image: {e:#}"),
                    }
                }
                let state = state(&player, &tiers, image_badge == Some(player.badge));
                if last.as_ref() != Some(&state) {
                    tracing::info!("rank {} ({:?} points)", state.badge, state.points);
                    store.set_rank(state.clone());
                    last = Some(state);
                }
            }
            Err(e) => tracing::warn!("rank: {e:#}"),
        }
        tokio::time::sleep(interval).await;
    }
}

fn state(player: &PlayerRank, tiers: &Tiers, has_image: bool) -> RankState {
    let points = player
        .last_match
        .as_ref()
        .and_then(|m| m.player_rank_final_flat_progress);
    RankState {
        badge: player.badge,
        tier: player.rank,
        subrank: player.subrank,
        points,
        names: tiers.names.get(&player.rank).cloned().unwrap_or_default(),
        color: tiers.colors.get(&player.rank).cloned().unwrap_or_default(),
        image: has_image.then(|| format!("/assets/rank/{}.webp", player.badge)),
        progress: points.and_then(|p| progress(player.rank, player.subrank, p)),
    }
}

/// 累積ポイントから、今のサブランクの中での進み具合を出す
fn progress(tier: u32, subrank: u32, points: u32) -> Option<RankProgress> {
    if tier == 0 || tier >= ETERNUS || !(1..=6).contains(&subrank) {
        return None;
    }
    let start = (tier - 1) * TIER_POINTS + (subrank - 1) * SUBRANK_POINTS;
    let max = if subrank == 6 {
        SUBRANK_POINTS * 2
    } else {
        SUBRANK_POINTS
    };
    Some(RankProgress {
        value: points.checked_sub(start)?.min(max),
        max,
    })
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

/// まだ取れていない言語の名前を取る。失敗した言語は警告して次の回に回す
async fn load_tiers(tiers: &mut Tiers, languages: &[LangCode]) {
    for code in languages {
        if tiers.loaded.contains(code) {
            continue;
        }
        match fetch_tiers(code).await {
            Ok(list) => {
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
                tiers.loaded.push(code.clone());
            }
            Err(e) => tracing::warn!("rank names ({code}): {e:#}"),
        }
    }
}

async fn fetch_tiers(code: &str) -> anyhow::Result<Vec<Tier>> {
    let api_name = lang::find(code).context("unknown language")?.api;
    Ok(crate::http()
        .get(format!("{API}/assets/ranks?language={api_name}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// サブランク付きのアイコン。サブランクのないティア（未ランク）はティアのアイコン
async fn fetch_image(player: &PlayerRank, tiers: &Tiers) -> anyhow::Result<Bytes> {
    let url = if player.subrank == 0 {
        tiers
            .plain_images
            .get(&player.rank)
            .cloned()
            .context("tier icon not loaded yet")?
    } else {
        format!(
            "{API}/assets/ranks/{}/{}/image?format=webp",
            player.rank, player.subrank
        )
    };
    Ok(crate::http()
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(badge: u32, points: Option<u32>) -> PlayerRank {
        serde_json::from_value(serde_json::json!({
            "badge": badge, "rank": badge / 10, "subrank": badge % 10,
            "last_match": points.map(|p| serde_json::json!({ "player_rank_final_flat_progress": p })),
        }))
        .unwrap()
    }

    #[test]
    fn state_uses_tier_names_and_final_progress() {
        let tiers = Tiers {
            names: BTreeMap::from([(7, BTreeMap::from([("ja".into(), "エミッサリー".into())]))]),
            colors: BTreeMap::from([(7, "#B47FEB".into())]),
            ..Tiers::default()
        };
        let s = state(&player(73, Some(44000)), &tiers, true);
        assert_eq!(s.points, Some(44000));
        assert_eq!(s.names["ja"], "エミッサリー");
        assert_eq!(s.image.as_deref(), Some("/assets/rank/73.webp"));
        assert_eq!(
            s.progress,
            Some(RankProgress {
                value: 0,
                max: 1000
            })
        );
    }

    #[test]
    fn no_image_url_until_the_icon_is_fetched() {
        let s = state(&player(73, Some(44000)), &Tiers::default(), false);
        assert_eq!(s.image, None);
    }

    #[test]
    fn progress_within_subrank() {
        // 7-3 の始まりは 6 ティア分（42000）+ 2 サブランク分（2000）
        assert_eq!(
            progress(7, 3, 44_650),
            Some(RankProgress {
                value: 650,
                max: 1000
            })
        );
        // 6 番目のサブランクは 2000
        assert_eq!(
            progress(1, 6, 5_500),
            Some(RankProgress {
                value: 500,
                max: 2000
            })
        );
        // エターナスと未ランクはゲージなし
        assert_eq!(progress(11, 2, 72_000), None);
        assert_eq!(progress(0, 0, 0), None);
        // 計算と合わないポイントは出さない
        assert_eq!(progress(7, 3, 1_000), None);
    }

    #[test]
    fn unranked_player_has_no_points() {
        let s = state(&player(0, None), &Tiers::default(), false);
        assert_eq!(s.points, None);
        assert_eq!(s.progress, None);
    }
}
