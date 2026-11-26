//! 画面の決まった場所の絵を、見本と比べて当てる（DESIGN.md 15 章「画面からの読み取り」）。
//! `heroes_eval` と `items_eval` が使う。枠に出る絵は、deadlock-api の画像を縮めて決まった位置に置いたものなので、
//! 枠ごとに全部の見本と比べて（色の正規化相互相関）、いちばん近いものを答えにする。

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use image::imageops::FilterType;
use image::{RgbImage, RgbaImage};
use serde::Deserialize;
use serde_json::json;

/// 配置は 1080p で測った。ほかの解像度では、画面の高さに比例させる（16:9 の 1080p でしか確かめていない）
pub const BASE_HEIGHT: f32 = 1080.0;
/// 置かれる位置のずれを、この画素数まで探す
const SHIFT: i32 = 2;

/// 測る画面。画面の一部を切り出した PNG と、枠ごとの正解
#[derive(Deserialize)]
pub struct Case {
    pub file: String,
    pub note: String,
    /// 切り出す前の画面の大きさ
    pub screen: (u32, u32),
    /// 切り出した左上の、画面での位置
    #[serde(default)]
    pub crop: (i32, i32),
    /// ショップを開いている画面か（アイテム欄の位置が変わる）
    #[serde(default)]
    pub shop: bool,
    /// 枠の順に、出ているものの名前。何もない枠は null
    pub names: Vec<Option<String>>,
}

impl Case {
    pub fn scale(&self) -> f32 {
        self.screen.1 as f32 / BASE_HEIGHT
    }
}

pub fn cases(path: &str) -> anyhow::Result<Vec<(Case, RgbImage)>> {
    let folder = Path::new(path).parent().context("cases folder")?;
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(path)?)?;
    cases
        .into_iter()
        .map(|case| {
            let frame = image::open(folder.join(&case.file))?.into_rgb8();
            Ok((case, frame))
        })
        .collect()
}

/// 見本の画像を取る（一度取ったら、置いてある分を使う）
pub async fn fetch(http: &reqwest::Client, url: &str, path: &Path) -> anyhow::Result<RgbaImage> {
    if !path.exists() {
        std::fs::create_dir_all(path.parent().context("folder")?)?;
        let bytes = http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        std::fs::write(path, bytes)?;
    }
    let image = image::open(path).with_context(|| format!("{}", path.display()))?;
    Ok(image.into_rgba8())
}

/// 縮めた見本のうち、比べる画素（見本の左上からの位置）と、その色（平均を引いて長さ 1 にしたもの）
pub struct Template {
    name: String,
    pixels: Vec<(u32, u32)>,
    colors: Vec<f32>,
}

impl Template {
    /// `image` を `size` に縮める。`compared` は、比べる範囲（幅と高さに対する割合で、画素の中心を渡す）。
    /// ほかの表示が重なるところを外す
    pub fn new(
        name: &str,
        image: &RgbaImage,
        size: (u32, u32),
        compared: impl Fn(f32, f32) -> bool,
    ) -> Self {
        let image = image::imageops::resize(image, size.0, size.1, FilterType::Lanczos3);
        let mut pixels = Vec::new();
        let mut colors = Vec::new();
        for (x, y, pixel) in image.enumerate_pixels() {
            let [r, g, b, a] = pixel.0;
            let at = (
                (x as f32 + 0.5) / size.0 as f32,
                (y as f32 + 0.5) / size.1 as f32,
            );
            // 見本の外（透明）は、画面では背景が透ける
            if a > 200 && compared(at.0, at.1) {
                pixels.push((x, y));
                colors.extend([r as f32, g as f32, b as f32]);
            }
        }
        normalize(&mut colors);
        Self {
            name: name.to_owned(),
            pixels,
            colors,
        }
    }

    /// 見本の左上を `(left, top)` に置いたときの近さ（-1〜1）。画面からはみ出すなら None
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
    let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
    values.iter_mut().for_each(|v| *v -= mean);
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
    values.iter_mut().for_each(|v| *v /= norm);
}

/// 枠にいちばん近い見本
pub struct Read {
    pub name: String,
    pub score: f32,
    /// その次に近い見本の近さ
    pub second: f32,
}

/// 見本の左上が `(left, top)` に来る枠を読む
pub fn read(frame: &RgbImage, templates: &[Template], left: i32, top: i32) -> Read {
    let mut buffer = Vec::new();
    let mut scores: Vec<(f32, &str)> = templates
        .iter()
        .map(|template| {
            let mut best = -1.0f32;
            for dy in -SHIFT..=SHIFT {
                for dx in -SHIFT..=SHIFT {
                    if let Some(score) = template.score(frame, left + dx, top + dy, &mut buffer) {
                        best = best.max(score);
                    }
                }
            }
            (best, template.name.as_str())
        })
        .collect();
    scores.sort_by(|a, b| b.0.total_cmp(&a.0));
    Read {
        name: scores[0].1.to_owned(),
        score: scores[0].0,
        second: scores[1].0,
    }
}

/// 読んだ結果を正解と比べて出し、`out` に書く。`threshold` より近くなければ「不明」にする
pub fn report(
    out: &str,
    threshold: f32,
    templates: usize,
    results: &[(&Case, Vec<Read>, Duration)],
) -> anyhow::Result<()> {
    let (mut correct, mut unknown, mut wrong) = (0, 0, 0);
    let mut cases = Vec::new();
    for (case, reads, elapsed) in results {
        anyhow::ensure!(
            case.names.len() == reads.len(),
            "{}: names は枠の数（{}）だけ書く（何もない枠は null）",
            case.file,
            reads.len()
        );
        println!("{}（{}）{elapsed:.0?}", case.file, case.note);
        let mut slots = Vec::new();
        for (slot, (expected, read)) in case.names.iter().zip(reads).enumerate() {
            let answer = (read.score >= threshold).then_some(read.name.as_str());
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
                "  {:>2} {:<18} → {:<18} {:.2}（次点 {:.2}）{verdict}",
                slot + 1,
                expected.as_deref().unwrap_or("-"),
                answer.unwrap_or("?"),
                read.score,
                read.second,
            );
            slots.push(json!({
                "expected": expected, "answer": answer, "best": read.name,
                "score": read.score, "second": read.second, "verdict": verdict,
            }));
        }
        cases.push(json!({ "file": case.file, "note": case.note, "slots": slots }));
    }
    println!("正解 {correct}　不明 {unknown}　間違い {wrong}（しきい値 {threshold}）");
    let result = json!({
        "threshold": threshold,
        "templates": templates,
        "correct": correct, "unknown": unknown, "wrong": wrong,
        "cases": cases,
    });
    std::fs::write(out, serde_json::to_string_pretty(&result)? + "\n")?;
    Ok(())
}
