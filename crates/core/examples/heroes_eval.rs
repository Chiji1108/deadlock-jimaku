//! 試合の画面の上部バーから、出ているヒーローを読めるかを測る（DESIGN.md 15 章「画面からの読み取り」）。
//! 上部バーの枠は、deadlock-api のカード画像（`icon_hero_card`）を縮めて、決まった位置に置いたもの。
//!
//! usage: cargo run -p overlays-core --release --example heroes_eval
//!   画面: experiments/data/top_bar/cases.json（画面の上の端を切り出した PNG と、左から順のヒーローの名前）
//!   出力: experiments/results/heroes_eval.json
//!   カード画像は deadlock-api から取り、`target/heroes_eval/` に置いておく

mod screen;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use screen::Template;
use serde::Deserialize;
use serde_json::Value;

const CASES: &str = "experiments/data/top_bar/cases.json";
const OUT: &str = "experiments/results/heroes_eval.json";
const CARDS: &str = "target/heroes_eval";
const API: &str = "https://api.deadlock-api.com/v1/assets/heroes";

/// 枠に出るカードの大きさ（1080p）。上の端は画面の上の端に付く
const CARD: (f32, f32) = (70.0, 95.0);
/// 隣の枠の中心までの幅
const SPACING: f32 = 88.0;
/// 画面の中央から、いちばん内側の枠の中心までの幅
const INNER: f32 = 146.0;
/// チームの枠の数。人数の少ないモード（ストリートブロール）は、内側から詰める
const TEAM: usize = 6;
/// これより近くなければ「不明」にする（死亡中の暗い枠、誰もいない枠、見た目を変える MOD）
const THRESHOLD: f32 = 0.8;

/// カードのうち、比べる範囲。左右はチームの印（「!」）が、下は丸い縁とソウルの数が重なる
fn compared(x: f32, y: f32) -> bool {
    (0.25..0.75).contains(&x) && y < 0.8
}

#[derive(Deserialize)]
struct Hero {
    id: u32,
    name: String,
    #[serde(default)]
    player_selectable: bool,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    images: BTreeMap<String, Value>,
}

/// 枠の中心の x 座標を、左の枠から順に
fn slots(screen_width: u32, scale: f32) -> Vec<f32> {
    let center = screen_width as f32 / 2.0;
    let offsets = (0..TEAM).map(|i| (INNER + SPACING * i as f32) * scale);
    let left = offsets.clone().rev().map(|offset| center - offset);
    left.chain(offsets.map(|offset| center + offset)).collect()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let http = reqwest::Client::new();
    let heroes: Vec<Hero> = http
        .get(API)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let mut cards = Vec::new();
    for hero in heroes {
        if !hero.player_selectable || hero.disabled {
            continue;
        }
        let Some(url) = hero.images.get("icon_hero_card").and_then(Value::as_str) else {
            continue;
        };
        let path = Path::new(CARDS).join(format!("{}.png", hero.id));
        cards.push((hero.name, screen::fetch(&http, url, &path).await?));
    }

    // 引数で、ほかの画面（cases.json）を測れる。結果はその隣に書く
    let given = std::env::args().nth(1);
    let out = given
        .as_ref()
        .map(|path| Path::new(path).with_extension("result.json"));
    let cases = screen::cases(given.as_deref().unwrap_or(CASES))?;
    let mut results = Vec::new();
    for (case, frame) in &cases {
        let scale = case.scale();
        let size = (
            (CARD.0 * scale).round() as u32,
            (CARD.1 * scale).round() as u32,
        );
        let templates: Vec<Template> = cards
            .iter()
            .map(|(name, card)| Template::new(name, card, size, compared))
            .collect();
        let started = Instant::now();
        let reads: Vec<_> = slots(case.screen.0, scale)
            .into_iter()
            .map(|center| {
                let left = (center - CARD.0 * scale / 2.0).round() as i32;
                screen::read(frame, &templates, left - case.crop.0, -case.crop.1)
            })
            .collect();
        results.push((case, reads, started.elapsed()));
    }
    screen::report(
        out.as_deref().unwrap_or(Path::new(OUT)),
        THRESHOLD,
        cards.len(),
        &results,
    )
}
