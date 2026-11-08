//! 配信者のランク（DESIGN.md 12 章）。deadlock-api を一定間隔で取りに行き、変わったら配信する。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use serde::Deserialize;

use crate::lang;
use crate::settings::Region;
use crate::store::Store;
use crate::types::{LangCode, LeaderboardRank, RankProgress, RankState};

const API: &str = "https://api.deadlock-api.com/v1";

/// サブランク 1 つ分のポイント（2026-11-05 のシーズン 2 から、6 番目は次のティアまで 1500）
const SUBRANK_POINTS: u32 = 1000;
const LAST_SUBRANK_POINTS: u32 = 1500;
const TIER_POINTS: u32 = SUBRANK_POINTS * 5 + LAST_SUBRANK_POINTS;
/// 最上位。上のティアがないのでゲージを出さず、リーダーボードの順位を出す
const ETERNUS: u32 = 11;
/// リーダーボードを取りに行く間隔（Valve の更新は 1 時間ごと）
const LEADERBOARD_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// これより古い順位は出さない（取れない状態が続いたら、ポイントに戻す）
const LEADERBOARD_MAX_AGE: Duration = Duration::from_secs(2 * 60 * 60);
/// 操作パネルのプレビューで、アカウントを入れる前に出す見本（エミッサリー III）
const SAMPLE: (u32, u32) = (7, 3);

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
    images: BTreeMap<String, Option<String>>,
}

/// ティアごとの名前（言語ごと）と、サブランクのないティアのアイコン
#[derive(Default)]
struct Tiers {
    names: BTreeMap<u32, BTreeMap<LangCode, String>>,
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
    let region = store.settings().rank_region;
    let mut tiers = Tiers::default();
    let mut last: Option<RankState> = None;
    // 最後に取れた順位と、取った時刻
    let mut placement: Option<(Option<u32>, Instant)> = None;
    // アイコンを持っているバッジ。取れなかったら次の回に取り直す
    let mut image_badge: Option<u32> = None;
    loop {
        load_tiers(&mut tiers, &languages).await;
        match fetch_rank(account_id).await {
            Ok(player) => {
                if image_badge != Some(player.badge) {
                    let url = || image_source(&player, &tiers);
                    match ensure_image(&store.rank_images(), player.badge, url).await {
                        Ok(()) => image_badge = Some(player.badge),
                        Err(e) => tracing::warn!("rank image: {e:#}"),
                    }
                }
                let mut state = state(&player, &tiers, image_badge == Some(player.badge));
                if let (ETERNUS, Some(region)) = (player.rank, region) {
                    // 取れてから時間が経ったら取り直す。取れなければ次の回にまた取りに行き、
                    // それまでは前の順位のまま（古くなったら出さない）
                    if placement.is_none_or(|(_, at)| at.elapsed() >= LEADERBOARD_INTERVAL) {
                        match leaderboard_rank(region, account_id).await {
                            Ok(rank) => placement = Some((rank, Instant::now())),
                            Err(e) => tracing::warn!("leaderboard: {e:#}"),
                        }
                    }
                    state.leaderboard = placement
                        .filter(|(_, at)| at.elapsed() < LEADERBOARD_MAX_AGE)
                        .and_then(|(rank, _)| rank)
                        .map(|rank| LeaderboardRank { region, rank });
                }
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
        // ゲージのないエターナスだけ累積のポイントを出す
        points: points.filter(|_| player.rank >= ETERNUS),
        names: tiers.names.get(&player.rank).cloned().unwrap_or_default(),
        image: has_image.then(|| image_url(player.badge)),
        progress: points.and_then(|p| progress(player.rank, player.subrank, p)),
        leaderboard: None,
    }
}

#[derive(Deserialize)]
struct Leaderboard {
    entries: Vec<LeaderboardEntry>,
}

#[derive(Deserialize)]
struct LeaderboardEntry {
    rank: u32,
    /// 表示名から推定したアカウント（同じ名前の人がいると複数になる）
    #[serde(default)]
    possible_account_ids: Vec<u32>,
}

/// その地域のリーダーボードでの順位。載っていなければ None（Valve の更新は 1 時間ごと）
async fn leaderboard_rank(region: Region, account_id: u32) -> anyhow::Result<Option<u32>> {
    let board: Leaderboard = crate::http()
        .get(format!("{API}/leaderboard/{}", region.as_api()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(find_rank(&board.entries, account_id))
}

/// 自分のアカウントだけが候補の行の順位。候補が何人もいる行は、同じ名前の別人かもしれないので使わない
/// （そのときは順位ではなくポイントを出す）
fn find_rank(entries: &[LeaderboardEntry], account_id: u32) -> Option<u32> {
    entries
        .iter()
        .find(|e| e.possible_account_ids == [account_id])
        .map(|e| e.rank)
}

/// 累積ポイントから、今のサブランクの中での進み具合を出す
fn progress(tier: u32, subrank: u32, points: u32) -> Option<RankProgress> {
    if tier == 0 || tier >= ETERNUS || !(1..=6).contains(&subrank) {
        return None;
    }
    let start = (tier - 1) * TIER_POINTS + (subrank - 1) * SUBRANK_POINTS;
    let max = if subrank == 6 {
        LAST_SUBRANK_POINTS
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
fn image_source(player: &PlayerRank, tiers: &Tiers) -> anyhow::Result<String> {
    if player.subrank == 0 {
        tiers
            .plain_images
            .get(&player.rank)
            .cloned()
            .context("tier icon not loaded yet")
    } else {
        Ok(subrank_image(player.rank, player.subrank))
    }
}

fn subrank_image(tier: u32, subrank: u32) -> String {
    format!("{API}/assets/ranks/{tier}/{subrank}/image?format=webp")
}

/// アイコンを配る URL（server.rs が `rank_images` の中から返す）
fn image_url(badge: u32) -> String {
    format!("/assets/rank/{badge}.webp")
}

pub fn image_path(dir: &Path, badge: u32) -> PathBuf {
    dir.join(format!("{badge}.webp"))
}

/// バッジのアイコンを保存しておく。保存したものがあれば取りに行かない
async fn ensure_image(
    dir: &Path,
    badge: u32,
    source: impl FnOnce() -> anyhow::Result<String>,
) -> anyhow::Result<()> {
    let path = image_path(dir, badge);
    if tokio::fs::try_exists(&path).await? {
        return Ok(());
    }
    let bytes = crate::http()
        .get(source()?)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    crate::write_atomic(&path, &bytes).await
}

/// プレビューの見本のアイコンを用意して、その URL を返す。本物のアイコンと同じく取ってきて保存する
pub async fn sample_image(store: &Store) -> anyhow::Result<String> {
    let (tier, subrank) = SAMPLE;
    let badge = tier * 10 + subrank;
    ensure_image(&store.rank_images(), badge, || {
        Ok(subrank_image(tier, subrank))
    })
    .await?;
    Ok(image_url(badge))
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
            ..Tiers::default()
        };
        let s = state(&player(73, Some(41000)), &tiers, true);
        assert_eq!(s.points, None);
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
    fn only_eternus_shows_points() {
        let tiers = Tiers::default();
        let eternus = state(&player(110, Some(72_000)), &tiers, false);
        assert_eq!(eternus.points, Some(72_000));
        let below_its_subrank = state(&player(43, Some(1_000)), &tiers, false);
        assert_eq!(below_its_subrank.points, None);
        assert_eq!(below_its_subrank.progress, None);
    }

    #[test]
    fn no_image_url_until_the_icon_is_fetched() {
        let s = state(&player(73, Some(44000)), &Tiers::default(), false);
        assert_eq!(s.image, None);
    }

    #[test]
    fn progress_within_subrank() {
        // 7-3 の始まりは 6 ティア分（39000）+ 2 サブランク分（2000）
        assert_eq!(
            progress(7, 3, 41_650),
            Some(RankProgress {
                value: 650,
                max: 1000
            })
        );
        // 6 番目のサブランクは 1500
        assert_eq!(
            progress(1, 6, 5_500),
            Some(RankProgress {
                value: 500,
                max: 1500
            })
        );
        // 2026-11-05 の実際の配信者: Sentinel III、22125 ポイント
        assert_eq!(
            progress(4, 3, 22_125),
            Some(RankProgress {
                value: 625,
                max: 1000
            })
        );
        // エターナスと未ランクはゲージなし
        assert_eq!(progress(11, 2, 72_000), None);
        assert_eq!(progress(0, 0, 0), None);
        // 計算と合わないポイントは出さない
        assert_eq!(progress(7, 3, 1_000), None);
    }

    #[test]
    fn leaderboard_prefers_entries_that_only_match_this_account() {
        let entry = |rank, ids: &[u32]| LeaderboardEntry {
            rank,
            possible_account_ids: ids.to_vec(),
        };
        let entries = [entry(5, &[1, 2, 3]), entry(9, &[2]), entry(12, &[4])];
        assert_eq!(find_rank(&entries, 2), Some(9));
        // 候補が何人もいる行だけなら、別人かもしれないので順位を出さない
        assert_eq!(find_rank(&entries, 1), None);
        assert_eq!(find_rank(&entries, 7), None);
    }

    /// 取りに来た回数を数え、決まった中身を返すサーバー。URL を返す
    async fn serve_image(hits: Arc<std::sync::atomic::AtomicU32>) -> String {
        use std::sync::atomic::Ordering;
        let app = axum::Router::new().route(
            "/image",
            axum::routing::get(move || async move {
                hits.fetch_add(1, Ordering::SeqCst);
                "webp"
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        format!("http://{addr}/image")
    }

    #[tokio::test]
    async fn an_icon_is_fetched_once_and_kept() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let hits = Arc::new(AtomicU32::new(0));
        let url = serve_image(hits.clone()).await;
        for _ in 0..2 {
            ensure_image(dir.path(), 73, || Ok(url.clone()))
                .await
                .unwrap();
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(image_path(dir.path(), 73)).unwrap(), b"webp");
    }

    #[tokio::test]
    async fn a_failed_fetch_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let unreachable = || Ok("http://127.0.0.1:9/image".to_owned());
        assert!(ensure_image(dir.path(), 73, unreachable).await.is_err());
        assert!(!image_path(dir.path(), 73).exists());
    }

    #[test]
    fn unranked_player_has_no_points() {
        let s = state(&player(0, None), &Tiers::default(), false);
        assert_eq!(s.points, None);
        assert_eq!(s.progress, None);
    }
}
