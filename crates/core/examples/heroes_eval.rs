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

use overlays_core::screen::heroes;
use serde::Deserialize;
use serde_json::Value;

const CASES: &str = "experiments/data/top_bar/cases.json";
const OUT: &str = "experiments/results/heroes_eval.json";
const CARDS: &str = "target/heroes_eval";
const API: &str = "https://api.deadlock-api.com/v1/assets/heroes";

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
    for (case, image) in &cases {
        let scale = case.scale();
        let templates = heroes::templates(
            cards.iter().map(|(name, card)| (name.as_str(), card)),
            scale,
        );
        let started = Instant::now();
        let reads = heroes::read(&case.frame(image), &templates, case.screen.0, scale);
        results.push((case, reads, started.elapsed()));
    }
    screen::report(
        out.as_deref().unwrap_or(Path::new(OUT)),
        heroes::THRESHOLD,
        cards.len(),
        &results,
    )
}
