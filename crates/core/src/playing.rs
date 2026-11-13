//! 配信者が今使っているヒーローを、試合から取る（DESIGN.md 9.13）。
//!
//! 「ウルト使う」のように、ヒーローの名前を言わずにスキルの話をすることが多い。使っているヒーローが分かれば、
//! そのスキルを候補にできる（`decide`）。ランクの表示につないである Steam のアカウントで、deadlock-api に聞く:
//! - 進行中の試合（`/matches/active`）。ゲームの「観戦」に並ぶ上位 200 試合だけなので、多くの人の試合は載らない
//! - 試合の履歴（`/players/{id}/match-history`）のいちばん新しい試合。終わった試合なので「前の試合」のヒーロー。
//!   同じヒーローを続けて使う人には当たる
//!
//! 外れていたら、プロフィールで選び直してもらう（設定の `playing`。選んであれば、ここは動かさない）

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;
use tokio::time::Instant;

use crate::store::Store;

const API: &str = "https://api.deadlock-api.com/v1";

/// 進行中の試合を見に行く間隔
const ACTIVE_INTERVAL: Duration = Duration::from_secs(60);
/// 試合の履歴を見に行く間隔。Steam につないだアカウントは、同じ所から 1 時間に 10 回までなので、控えめにする
const HISTORY_INTERVAL: Duration = Duration::from_secs(10 * 60);
const TIMEOUT: Duration = Duration::from_secs(15);

/// 設定にアカウントがあり、ヒーローを自分で選んでいなければ、取りに行き続ける
/// （用語の判定に使うので、Deadlock モードのときだけ）
pub async fn run_configured(store: Arc<Store>) {
    let settings = store.settings();
    let account = settings.rank_account.filter(|_| settings.deadlock_mode);
    let (Some(account), None) = (account, &settings.playing) else {
        store.set_played(None);
        return;
    };
    // 進行中の試合で分からない間は、前の試合のヒーロー
    let mut last: Option<u32> = None;
    let mut asked_history: Option<Instant> = None;
    loop {
        if asked_history.is_none_or(|at| at.elapsed() >= HISTORY_INTERVAL) {
            asked_history = Some(Instant::now());
            match last_hero(API, account).await {
                Ok(hero) => last = hero.or(last),
                Err(e) => tracing::debug!("match history: {e:#}"),
            }
        }
        let now = match active_hero(API, account).await {
            Ok(hero) => hero,
            Err(e) => {
                tracing::debug!("active matches: {e:#}");
                None
            }
        };
        store.set_played(now.or(last).map(|id| format!("hero:{id}")));
        tokio::time::sleep(ACTIVE_INTERVAL).await;
    }
}

#[derive(Deserialize)]
struct Match {
    players: Vec<Player>,
}

#[derive(Deserialize)]
struct Player {
    account_id: u32,
    hero_id: u32,
}

/// 進行中の試合で使っているヒーロー。試合が一覧に載っていなければ None
async fn active_hero(api: &str, account: u32) -> anyhow::Result<Option<u32>> {
    let url = format!("{api}/matches/active?account_id={account}");
    let res = crate::http().get(url).timeout(TIMEOUT).send().await?;
    let matches: Vec<Match> = res.error_for_status()?.json().await?;
    let players = matches.into_iter().flat_map(|m| m.players);
    Ok(players
        .into_iter()
        .find(|player| player.account_id == account)
        .map(|player| player.hero_id))
}

#[derive(Deserialize)]
struct Played {
    start_time: u64,
    hero_id: u32,
}

/// いちばん新しい試合で使ったヒーロー。履歴がなければ None
async fn last_hero(api: &str, account: u32) -> anyhow::Result<Option<u32>> {
    let url = format!("{api}/players/{account}/match-history");
    let res = crate::http().get(url).timeout(TIMEOUT).send().await?;
    let history: Vec<Played> = res
        .error_for_status()?
        .json()
        .await
        .context("match history")?;
    Ok(history
        .into_iter()
        .max_by_key(|played| played.start_time)
        .map(|played| played.hero_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::routing::get;
    use serde_json::json;

    /// 進行中の試合に自分がいればそのヒーロー、履歴はいちばん新しい試合のヒーロー
    #[tokio::test]
    async fn the_hero_is_read_from_the_active_match_and_the_history() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let active = || async {
            Json(json!([{ "match_id": 1, "players": [
                { "account_id": 7, "hero_id": 13, "team": 0 },
                { "account_id": 8, "hero_id": 20, "team": 1 },
            ]}]))
        };
        let history = || async {
            Json(json!([
                { "match_id": 3, "start_time": 300, "hero_id": 19 },
                { "match_id": 9, "start_time": 900, "hero_id": 84 },
                { "match_id": 5, "start_time": 500, "hero_id": 10 },
            ]))
        };
        let router = axum::Router::new()
            .route("/matches/active", get(active))
            .route("/players/{account}/match-history", get(history));
        tokio::spawn(async move { axum::serve(listener, router).await });
        assert_eq!(active_hero(&api, 8).await.unwrap(), Some(20));
        // 一覧に載っていない人（上位 200 試合の外）
        assert_eq!(active_hero(&api, 99).await.unwrap(), None);
        assert_eq!(last_hero(&api, 8).await.unwrap(), Some(84));
    }
}
