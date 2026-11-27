//! 試合の画面の上の中央の時計を読めるかを測る（DESIGN.md 18.4）。
//!
//! usage: cargo run -p overlays-core --release --example clock_eval
//!   画面: experiments/data/top_bar_match/cases.json（上の端を切り出した PNG と、時計（"8:48" の形。隠れていれば null））

use std::path::Path;

use overlays_core::screen::{BASE_HEIGHT, Frame, clock};
use serde::Deserialize;

const CASES: &str = "experiments/data/top_bar_match/cases.json";

#[derive(Deserialize)]
struct Case {
    file: String,
    screen: (u32, u32),
    clock: Option<String>,
    #[serde(default)]
    spectating: bool,
}

fn main() -> anyhow::Result<()> {
    let folder = Path::new(CASES).parent().expect("cases folder");
    let cases: Vec<Case> = serde_json::from_slice(&std::fs::read(CASES)?)?;
    let (mut correct, mut wrong) = (0, 0);
    for case in &cases {
        let image = image::open(folder.join(&case.file))?.into_rgb8();
        let scale = case.screen.1 as f32 / BASE_HEIGHT;
        let frame = Frame {
            image: &image,
            origin: (0, 0),
        };
        let templates = clock::templates(scale);
        let read = clock::read(&frame, &templates, case.screen.0, scale, case.spectating);
        let expected = case.clock.as_deref().map(|text| {
            let (minutes, seconds) = text.split_once(':').expect("m:ss");
            minutes.parse::<u32>().expect("minutes") * 60 + seconds.parse::<u32>().expect("seconds")
        });
        let verdict = if read == expected { "correct" } else { "wrong" };
        match read == expected {
            true => correct += 1,
            false => wrong += 1,
        }
        println!(
            "{:<22} {:<6} → {:?} {verdict}",
            case.file,
            case.clock.as_deref().unwrap_or("-"),
            read
        );
    }
    println!("正解 {correct}　間違い {wrong}");
    Ok(())
}
