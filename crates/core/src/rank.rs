//! 配信者のランク（DESIGN.md 12 章）。deadlock-api を一定間隔で取りに行き、変わったら配信する。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, ensure};
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
    // アイコンを持っているバッジと、配る URL。取れなかったら次の回に取り直す
    let mut image: Option<(u32, String)> = None;
    loop {
        load_tiers(&mut tiers, &languages).await;
        match fetch_rank(account_id).await {
            Ok(player) => {
                if image
                    .as_ref()
                    .is_none_or(|(badge, _)| *badge != player.badge)
                {
                    match player_image(&store.rank_images(), &player, &tiers).await {
                        Ok(url) => image = Some((player.badge, url)),
                        Err(e) => tracing::warn!("rank image: {e:#}"),
                    }
                }
                let current = image
                    .as_ref()
                    .filter(|(badge, _)| *badge == player.badge)
                    .map(|(_, url)| url.clone());
                let mut state = state(&player, &tiers, current);
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

fn state(player: &PlayerRank, tiers: &Tiers, image: Option<String>) -> RankState {
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
        image,
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

/// 保存と配信の名前。元の画像の URL ごとに別の名前にするので、同じ名前の中身は変わらない
fn image_name(badge: u32, source: &str) -> String {
    format!("{badge}-{}.webp", &crate::sha256(source.as_bytes())[..16])
}

/// 配る URL の名前から、保存した場所を出す（server.rs）。`image_name` の形の名前だけを受け付ける
pub fn image_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let (badge, hash) = name.strip_suffix(".webp")?.split_once('-')?;
    let valid = !badge.is_empty()
        && badge.bytes().all(|b| b.is_ascii_digit())
        && hash.len() == 16
        && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    valid.then(|| dir.join(name))
}

/// RIFF の WebP か。エラーのページなどを画像として保存しない
fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
}

/// バッジのアイコンを保存して、配る URL を返す。同じ URL から取ったものがあれば取りに行かない。
/// 新しく保存したら、そのバッジの前のアイコンは消す
async fn ensure_image(dir: &Path, badge: u32, source: &str) -> anyhow::Result<String> {
    let name = image_name(badge, source);
    let path = dir.join(&name);
    if !tokio::fs::try_exists(&path).await? {
        let bytes = crate::http()
            .get(source)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        ensure!(is_webp(&bytes), "{source} is not a webp image");
        crate::write_atomic(&path, &bytes).await?;
        if let Err(e) = remove_old_images(dir, badge, &name).await {
            tracing::warn!("rank image cleanup: {e:#}");
        }
    }
    Ok(format!("/assets/rank/{name}"))
}

/// 元の URL が変わる前のアイコンと、URL を名前に含めていなかった頃の `{badge}.webp`
async fn remove_old_images(dir: &Path, badge: u32, kept: &str) -> std::io::Result<()> {
    let mut files = tokio::fs::read_dir(dir).await?;
    while let Some(file) = files.next_entry().await? {
        let name = file.file_name();
        let Some(name) = name.to_str() else { continue };
        let old = name.strip_suffix(".webp").is_some_and(|stem| {
            stem == badge.to_string() || stem.starts_with(&format!("{badge}-"))
        });
        if old && name != kept {
            tokio::fs::remove_file(file.path()).await?;
        }
    }
    Ok(())
}

async fn player_image(dir: &Path, player: &PlayerRank, tiers: &Tiers) -> anyhow::Result<String> {
    ensure_image(dir, player.badge, &image_source(player, tiers)?).await
}

/// プレビューの見本のアイコンを用意して、その URL を返す。本物のアイコンと同じく取ってきて保存する
pub async fn sample_image(store: &Store) -> anyhow::Result<String> {
    let (tier, subrank) = SAMPLE;
    ensure_image(
        &store.rank_images(),
        tier * 10 + subrank,
        &subrank_image(tier, subrank),
    )
    .await
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
        let image = "/assets/rank/73-0123456789abcdef.webp".to_owned();
        let s = state(&player(73, Some(41000)), &tiers, Some(image.clone()));
        assert_eq!(s.points, None);
        assert_eq!(s.names["ja"], "エミッサリー");
        assert_eq!(s.image, Some(image));
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
        let eternus = state(&player(110, Some(72_000)), &tiers, None);
        assert_eq!(eternus.points, Some(72_000));
        let below_its_subrank = state(&player(43, Some(1_000)), &tiers, None);
        assert_eq!(below_its_subrank.points, None);
        assert_eq!(below_its_subrank.progress, None);
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

    const WEBP: &[u8] = b"RIFF\x10\0\0\0WEBPVP8 \0\0\0\0";

    /// 取りに来た回数を数え、`/a` と `/b` は WebP、`/html` は HTML を返すサーバー。元の URL を返す
    async fn serve_images(hits: Arc<std::sync::atomic::AtomicU32>) -> String {
        use axum::extract::Path as UrlPath;
        use std::sync::atomic::Ordering;
        let app = axum::Router::new().route(
            "/{name}",
            axum::routing::get(move |UrlPath(name): UrlPath<String>| async move {
                hits.fetch_add(1, Ordering::SeqCst);
                if name == "html" {
                    b"<html>rate limited</html>".to_vec()
                } else {
                    WEBP.to_vec()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        format!("http://{addr}")
    }

    fn saved(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|file| file.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn served_file(dir: &Path, url: &str) -> PathBuf {
        let name = url.strip_prefix("/assets/rank/").unwrap();
        image_path(dir, name).unwrap()
    }

    #[tokio::test]
    async fn an_icon_is_fetched_once_per_source_and_kept() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let hits = Arc::new(AtomicU32::new(0));
        let origin = serve_images(hits.clone()).await;
        let a = format!("{origin}/a");
        let first = ensure_image(dir.path(), 73, &a).await.unwrap();
        assert_eq!(ensure_image(dir.path(), 73, &a).await.unwrap(), first);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            std::fs::read(served_file(dir.path(), &first)).unwrap(),
            WEBP
        );
    }

    #[tokio::test]
    async fn a_new_source_gets_a_new_file_and_drops_the_old_one() {
        use std::sync::atomic::AtomicU32;
        let dir = tempfile::tempdir().unwrap();
        let origin = serve_images(Arc::new(AtomicU32::new(0))).await;
        std::fs::write(dir.path().join("73.webp"), WEBP).unwrap();
        std::fs::write(dir.path().join("7.webp"), WEBP).unwrap();
        let other = ensure_image(dir.path(), 74, &format!("{origin}/a"))
            .await
            .unwrap();
        let old = ensure_image(dir.path(), 73, &format!("{origin}/a"))
            .await
            .unwrap();
        let new = ensure_image(dir.path(), 73, &format!("{origin}/b"))
            .await
            .unwrap();
        assert_ne!(old, new);
        let mut kept = vec![
            "7.webp".to_owned(),
            other.replace("/assets/rank/", ""),
            new.replace("/assets/rank/", ""),
        ];
        kept.sort();
        assert_eq!(saved(dir.path()), kept);
    }

    #[tokio::test]
    async fn only_a_webp_image_is_saved() {
        use std::sync::atomic::AtomicU32;
        let dir = tempfile::tempdir().unwrap();
        let origin = serve_images(Arc::new(AtomicU32::new(0))).await;
        assert!(
            ensure_image(dir.path(), 73, &format!("{origin}/html"))
                .await
                .is_err()
        );
        assert!(saved(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn a_failed_fetch_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let unreachable = "http://127.0.0.1:9/image";
        assert!(ensure_image(dir.path(), 73, unreachable).await.is_err());
        assert!(!dir.path().join(image_name(73, unreachable)).exists());
    }

    #[test]
    fn only_saved_icon_names_reach_the_folder() {
        let dir = Path::new("rank");
        assert_eq!(
            image_path(dir, "73-0123456789abcdef.webp"),
            Some(dir.join("73-0123456789abcdef.webp"))
        );
        for name in [
            "73.webp",
            "73-0123456789abcdef.png",
            "73-0123456789ABCDEF.webp",
            "73-0123456789abcde.webp",
            "-0123456789abcdef.webp",
            "+73-0123456789abcdef.webp",
            "../73-0123456789abcdef.webp",
        ] {
            assert_eq!(image_path(dir, name), None, "{name}");
        }
    }

    #[test]
    fn unranked_player_has_no_points() {
        let s = state(&player(0, None), &Tiers::default(), None);
        assert_eq!(s.points, None);
        assert_eq!(s.progress, None);
    }
}
