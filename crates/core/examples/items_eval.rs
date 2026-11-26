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

use screen::Template;
use serde::Deserialize;

const CASES: &str = "experiments/data/item_slots/cases.json";
const OUT: &str = "experiments/results/items_eval.json";
const IMAGES: &str = "target/items_eval";
const API: &str = "https://api.deadlock-api.com/v1/assets/items";

/// 枠の大きさ（1080p）
const SLOT: f32 = 54.0;
/// 隣の枠までの幅（横も縦も）
const SPACING: f32 = 62.0;
/// 左上の枠の、画面での位置
const FIRST: (f32, f32) = (26.0, 946.0);
/// ショップを開いている間は、欄がこれだけ上にずれる
const SHOP_RISE: f32 = 40.0;
/// 枠の並び（横、縦）。鍵の掛かった枠も数える
const GRID: (usize, usize) = (6, 2);
/// これより近くなければ「不明」にする（空の枠、鍵の掛かった枠、クールダウンで暗い枠）。
/// 空の枠は 0.73 まで上がり、暗くない枠は 0.83 まで下がる（Disarming Hex）ので、その間に置く
const THRESHOLD: f32 = 0.8;

/// 枠のうち、比べる範囲。右上は段の印（III など）が重なり、縁は枠の線が掛かる
fn compared(x: f32, y: f32) -> bool {
    let inside = |v: f32| (0.06..0.94).contains(&v);
    inside(x) && inside(y) && x - y < 0.35
}

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

/// 枠の近さの合計
fn total(reads: &[screen::Read]) -> f32 {
    reads.iter().map(|read| read.score).sum()
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
    for (case, frame) in &cases {
        let scale = case.scale();
        let size = (SLOT * scale).round() as u32;
        let templates: Vec<Template> = images
            .iter()
            .map(|(name, image)| Template::new(name, image, (size, size), compared))
            .collect();
        let started = Instant::now();
        // ショップを開いているかは分からないので、両方の位置を読んで、よく合うほうを採る
        let reads = [0.0, SHOP_RISE]
            .map(|rise| {
                let mut reads = Vec::new();
                for row in 0..GRID.1 {
                    for column in 0..GRID.0 {
                        let left = (FIRST.0 + SPACING * column as f32) * scale;
                        let top = (FIRST.1 - rise + SPACING * row as f32) * scale;
                        reads.push(screen::read(
                            frame,
                            &templates,
                            left.round() as i32 - case.crop.0,
                            top.round() as i32 - case.crop.1,
                        ));
                    }
                }
                reads
            })
            .into_iter()
            .max_by(|a, b| total(a).total_cmp(&total(b)))
            .expect("two positions");
        results.push((case, reads, started.elapsed()));
    }
    screen::report(
        out.as_deref().unwrap_or(Path::new(OUT)),
        THRESHOLD,
        images.len(),
        &results,
    )
}
