//! 画面の下の中央のアビリティの絵から、自分のヒーローを当てられるかを測る（DESIGN.md 18.4）。
//!
//! usage: cargo run -p overlays-core --release --example abilities_eval
//!   画面: experiments/data/whole/cases.json（画面の全体の PNG と、自分のヒーローの英語の名前）
//!   絵は deadlock-api から取り、`target/screen_samples/` に置いておく

use std::path::Path;
use std::time::Instant;

use overlays_core::screen::{BASE_HEIGHT, Frame, abilities, samples};
use serde::Deserialize;

const CASES: &str = "experiments/data/whole/cases.json";

#[derive(Deserialize)]
struct Case {
    file: String,
    screen: (u32, u32),
    hero: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let samples = samples::load(Path::new("target/screen_samples"), None).await?;
    let folder = Path::new(CASES).parent().expect("cases folder");
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(CASES)?)?;
    let (mut correct, mut unknown, mut wrong) = (0, 0, 0);
    for case in &cases {
        let image = image::open(folder.join(&case.file))?.into_rgb8();
        let scale = case.screen.1 as f32 / BASE_HEIGHT;
        // 正解は、英語の名前で書いてある
        let named = (samples.abilities.iter()).map(|sample| (sample.name.as_str(), &sample.image));
        let shapes = abilities::shapes(named, scale);
        let [left, top, width, height] = abilities::area(case.screen, scale);
        let cut = image::imageops::crop_imm(&image, left, top, width, height).to_image();
        let frame = Frame {
            image: &cut,
            origin: (left as i32, top as i32),
        };
        let started = Instant::now();
        let reads = abilities::read(&frame, &shapes, case.screen.0, scale);
        let elapsed = started.elapsed();
        let hero = abilities::hero(&reads);
        let verdict = match hero {
            Some(hero) if hero == case.hero => "correct",
            Some(_) => "wrong",
            None => "unknown",
        };
        match verdict {
            "correct" => correct += 1,
            "unknown" => unknown += 1,
            _ => wrong += 1,
        }
        let reads: Vec<String> = (reads.iter())
            .map(|(name, score)| format!("{name} {score:.2}"))
            .collect();
        println!(
            "{:<30} {:<12} → {:<12} {verdict} {elapsed:.0?}（{}）",
            case.file,
            case.hero,
            hero.unwrap_or("?"),
            reads.join("、")
        );
    }
    println!("正解 {correct}　不明 {unknown}　間違い {wrong}");
    Ok(())
}
