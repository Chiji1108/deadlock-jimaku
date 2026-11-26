//! 試合の画面の上部バーから、出ているヒーローを読めるかを測る（DESIGN.md 15 章「画面からの読み取り」）。
//! 上部バーの枠は、deadlock-api のカード画像（`icon_hero_card`）を縮めて、決まった位置に置いたもの。
//! なので、枠ごとに全ヒーローのカードと比べて（正規化相互相関）、いちばん近いものを答えにする。
//!
//! usage: cargo run -p overlays-core --release --example heroes_eval
//!   画面: experiments/data/top_bar/cases.json（画面の上の端を切り出した PNG と、左から順のヒーローの名前）
//!   出力: experiments/results/heroes_eval.json
//!   カード画像は deadlock-api から取り、`target/heroes_eval/` に置いておく

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use anyhow::Context;
use image::imageops::FilterType;
use image::{RgbImage, RgbaImage};
use serde::Deserialize;
use serde_json::{Value, json};

const CASES: &str = "experiments/data/top_bar/cases.json";
const OUT: &str = "experiments/results/heroes_eval.json";
const CARDS: &str = "target/heroes_eval";
const API: &str = "https://api.deadlock-api.com/v1/assets/heroes";

/// 配置は 1080p で測った。ほかの解像度では、画面の高さに比例させる（16:9 の 1080p でしか確かめていない）
const BASE_HEIGHT: f32 = 1080.0;
/// 枠に出るカードの大きさ。上の端は画面の上の端に付く
const CARD: (f32, f32) = (70.0, 95.0);
/// 隣の枠の中心までの幅
const SPACING: f32 = 88.0;
/// 画面の中央から、いちばん内側の枠の中心までの幅
const INNER: f32 = 146.0;
/// チームの枠の数。人数の少ないモード（ストリートブロール）は、内側から詰める
const TEAM: usize = 6;
/// カードのうち、比べる範囲（幅と高さに対する割合）。左右はチームの印（「!」）が、下は丸い縁とソウルの数が重なる
const COMPARE_X: (f32, f32) = (0.25, 0.75);
const COMPARE_BOTTOM: f32 = 0.8;
/// 置かれる位置のずれを、この画素数まで探す
const SHIFT: i32 = 2;
/// これより近くなければ「不明」にする（死亡中の暗い枠、誰もいない枠、見た目を変える MOD）
const THRESHOLD: f32 = 0.8;

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

#[derive(Deserialize)]
struct Case {
    file: String,
    note: String,
    /// 切り出す前の画面の大きさ
    screen: (u32, u32),
    /// 左の枠から順に。誰もいない枠は null
    heroes: Vec<Option<String>>,
}

/// 縮めたカードのうち、比べる画素（カードの左上からの位置）と、その色（平均を引いて長さ 1 にしたもの）
struct Template {
    name: String,
    pixels: Vec<(u32, u32)>,
    colors: Vec<f32>,
}

impl Template {
    fn new(name: String, card: &RgbaImage, scale: f32) -> Self {
        let (width, height) = (
            (CARD.0 * scale).round() as u32,
            (CARD.1 * scale).round() as u32,
        );
        let card = image::imageops::resize(card, width, height, FilterType::Lanczos3);
        let (left, right) = (
            (width as f32 * COMPARE_X.0) as u32,
            (width as f32 * COMPARE_X.1) as u32,
        );
        let bottom = (height as f32 * COMPARE_BOTTOM) as u32;
        let mut pixels = Vec::new();
        let mut colors = Vec::new();
        for y in 0..bottom {
            for x in left..right {
                let [r, g, b, a] = card.get_pixel(x, y).0;
                // カードの外（透明）は、画面では背景が透ける
                if a > 200 {
                    pixels.push((x, y));
                    colors.extend([r as f32, g as f32, b as f32]);
                }
            }
        }
        normalize(&mut colors);
        Self {
            name,
            pixels,
            colors,
        }
    }

    /// カードの左上を `(left, top)` に置いたときの近さ（-1〜1）。画面からはみ出すなら None
    fn score(&self, frame: &RgbImage, left: i32, top: i32, buffer: &mut Vec<f32>) -> Option<f32> {
        buffer.clear();
        for &(x, y) in &self.pixels {
            let (x, y) = (left + x as i32, top + y as i32);
            if x < 0 || y < 0 || x >= frame.width() as i32 || y >= frame.height() as i32 {
                return None;
            }
            buffer.extend(frame.get_pixel(x as u32, y as u32).0.map(f32::from));
        }
        normalize(buffer);
        Some(
            self.colors
                .iter()
                .zip(buffer.iter())
                .map(|(a, b)| a * b)
                .sum(),
        )
    }
}

/// 平均を引いて、長さを 1 にする
fn normalize(values: &mut [f32]) {
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    values.iter_mut().for_each(|v| *v -= mean);
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
    values.iter_mut().for_each(|v| *v /= norm);
}

/// 枠の中心の x 座標を、左の枠から順に
fn slots(screen_width: u32, scale: f32) -> Vec<f32> {
    let center = screen_width as f32 / 2.0;
    let offsets = (0..TEAM).map(|i| (INNER + SPACING * i as f32) * scale);
    let left = offsets.clone().rev().map(|offset| center - offset);
    left.chain(offsets.map(|offset| center + offset)).collect()
}

/// 枠にいちばん近いヒーローと近さ、その次の近さ
fn read<'a>(
    frame: &RgbImage,
    templates: &'a [Template],
    center: f32,
    scale: f32,
) -> (&'a str, f32, f32) {
    let left = (center - CARD.0 * scale / 2.0).round() as i32;
    let mut buffer = Vec::new();
    let mut scores: Vec<(f32, &str)> = templates
        .iter()
        .map(|template| {
            let mut best = -1.0f32;
            for dy in -SHIFT..=SHIFT {
                for dx in -SHIFT..=SHIFT {
                    if let Some(score) = template.score(frame, left + dx, dy, &mut buffer) {
                        best = best.max(score);
                    }
                }
            }
            (best, template.name.as_str())
        })
        .collect();
    scores.sort_by(|a, b| b.0.total_cmp(&a.0));
    (scores[0].1, scores[0].0, scores[1].0)
}

/// 選べるヒーローのカード画像を取る（一度取ったら、置いてある分を使う）
async fn cards() -> anyhow::Result<Vec<(String, RgbaImage)>> {
    let http = reqwest::Client::new();
    let heroes: Vec<Hero> = http
        .get(API)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    std::fs::create_dir_all(CARDS)?;
    let mut cards = Vec::new();
    for hero in heroes {
        if !hero.player_selectable || hero.disabled {
            continue;
        }
        let Some(url) = hero.images.get("icon_hero_card").and_then(Value::as_str) else {
            continue;
        };
        let path = Path::new(CARDS).join(format!("{}.png", hero.id));
        if !path.exists() {
            let bytes = http
                .get(url)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;
            std::fs::write(&path, bytes)?;
        }
        let card = image::open(&path).with_context(|| format!("{}", path.display()))?;
        cards.push((hero.name, card.into_rgba8()));
    }
    Ok(cards)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cards = cards().await?;
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(CASES)?)?;
    let folder = Path::new(CASES).parent().context("cases folder")?;

    let (mut correct, mut unknown, mut wrong) = (0, 0, 0);
    let mut results = Vec::new();
    for case in &cases {
        let scale = case.screen.1 as f32 / BASE_HEIGHT;
        let templates: Vec<Template> = cards
            .iter()
            .map(|(name, card)| Template::new(name.clone(), card, scale))
            .collect();
        let frame = image::open(folder.join(&case.file))?.into_rgb8();
        let started = Instant::now();
        let reads: Vec<_> = slots(case.screen.0, scale)
            .into_iter()
            .map(|center| read(&frame, &templates, center, scale))
            .collect();
        let elapsed = started.elapsed();

        println!("{}（{}）{elapsed:.0?}", case.file, case.note);
        let mut slots = Vec::new();
        for (slot, (expected, (best, score, second))) in case.heroes.iter().zip(reads).enumerate() {
            let answer = (score >= THRESHOLD).then_some(best);
            let verdict = match (answer, expected.as_deref()) {
                (Some(answer), Some(expected)) if answer == expected => "correct",
                (None, None) => "correct",
                (None, Some(_)) => "unknown",
                (Some(_), _) => "wrong",
            };
            match verdict {
                "correct" => correct += 1,
                "unknown" => unknown += 1,
                _ => wrong += 1,
            }
            println!(
                "  {:>2} {:<12} → {:<12} {score:.2}（次点 {second:.2}）{verdict}",
                slot + 1,
                expected.as_deref().unwrap_or("-"),
                answer.unwrap_or("?"),
            );
            slots.push(json!({
                "expected": expected, "answer": answer, "best": best,
                "score": score, "second": second, "verdict": verdict,
            }));
        }
        results.push(json!({ "file": case.file, "note": case.note, "slots": slots }));
    }
    println!("正解 {correct}　不明 {unknown}　間違い {wrong}（しきい値 {THRESHOLD}）");
    let out = json!({
        "threshold": THRESHOLD,
        "heroes": cards.len(),
        "correct": correct, "unknown": unknown, "wrong": wrong,
        "cases": results,
    });
    std::fs::write(OUT, serde_json::to_string_pretty(&out)? + "\n")?;
    Ok(())
}
