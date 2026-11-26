//! 試合の画面の左下のアイテム欄から、持っているアイテムを読めるかを測る（DESIGN.md 15 章「画面からの読み取り」）。
//! 枠は、deadlock-api のショップの画像（`shop_image`）を縮めて、決まった位置に置いたもの。
//! 前に読んだときになかったアイテムが、買ったアイテムになる。
//!
//! usage: cargo run -p overlays-core --release --example items_eval
//!   画面: experiments/data/item_slots/cases.json（画面の左下を切り出した PNG と、上の段の左から順のアイテムの名前）
//!   出力: experiments/results/items_eval.json
//!   ショップの画像は deadlock-api から取り、`target/items_eval/` に置いておく

mod screen;

use std::path::Path;
use std::time::Instant;

use overlays_core::screen::items;
use serde::Deserialize;

const CASES: &str = "experiments/data/item_slots/cases.json";
const OUT: &str = "experiments/results/items_eval.json";
const IMAGES: &str = "target/items_eval";
const API: &str = "https://api.deadlock-api.com/v1/assets/items";

#[derive(Deserialize)]
struct Item {
    id: u32,
    name: Option<String>,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    shopable: bool,
    shop_image: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let http = reqwest::Client::new();
    let items: Vec<Item> = http
        .get(API)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let mut images = Vec::new();
    for item in items {
        if item.kind != "upgrade" || !item.shopable {
            continue;
        }
        let (Some(name), Some(url)) = (item.name, item.shop_image) else {
            continue;
        };
        let path = Path::new(IMAGES).join(format!("{}.png", item.id));
        images.push((name, screen::fetch(&http, &url, &path).await?));
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
        let templates = items::templates(
            images.iter().map(|(name, image)| (name.as_str(), image)),
            scale,
        );
        let started = Instant::now();
        let slots = items::read(&case.frame(image), &templates, scale);
        results.push((case, slots.reads, started.elapsed()));
    }
    screen::report(
        out.as_deref().unwrap_or(Path::new(OUT)),
        items::THRESHOLD,
        images.len(),
        &results,
    )
}
