//! 画面を読めるかを測る例（`heroes_eval`、`items_eval`）が分け合う部分（DESIGN.md 15 章「画面からの読み取り」）。
//! 測る画面と正解を読み、読んだ結果を正解と比べて出す。見本を取るのも、読むのも、本体の `overlays_core::screen`。

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use image::RgbImage;
use overlays_core::screen::{BASE_HEIGHT, Frame, Read};
use serde::Deserialize;
use serde_json::json;

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
    /// 枠の順に、出ているものの名前。何もない枠は null
    pub names: Vec<Option<String>>,
}

impl Case {
    pub fn scale(&self) -> f32 {
        self.screen.1 as f32 / BASE_HEIGHT
    }

    pub fn frame<'a>(&self, image: &'a RgbImage) -> Frame<'a> {
        Frame {
            image,
            origin: self.crop,
        }
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

/// 見本の画像を置く場所（一度取ったら、置いてある分を使う）
pub const SAMPLES: &str = "target/screen_samples";

/// 読んだ結果を正解と比べて出し、`out` に書く。`threshold` より近くなければ「不明」にする
pub fn report(
    out: &Path,
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
            let answer = read.answer(threshold);
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
