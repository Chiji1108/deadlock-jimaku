//! ゲームの画面を取り込めるかを確かめる（DESIGN.md 15 章「画面からの読み取り」）。
//! 本体の取り込み（`screen/capture.rs`）で画面の全体を読み、PNG に書いて、掛かった時間を出す。
//!
//! usage: cargo run -p overlays-core --release --example capture -- <出力.png> [枚数] [間隔ミリ秒]
//!   枚数が 2 以上なら、`出力-1.png` のように番号を付けて書く
//!   `OBS_INPUT=<ソースの名前>` を付けると、Deadlock のウィンドウ（Windows だけ）ではなく、OBS のソースから読む
//!   （`OBS_PORT`、`OBS_PASSWORD` も渡せる。DESIGN.md 18.8）

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Context;
use overlays_core::screen::capture::{Capture, Feed};
use overlays_core::screen::obs;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let out = args.next().context("出力の PNG を渡す")?;
    let count: usize = args.next().map(|v| v.parse()).transpose()?.unwrap_or(1);
    let interval =
        Duration::from_millis(args.next().map(|v| v.parse()).transpose()?.unwrap_or(1000));

    let started = Instant::now();
    let feed = match std::env::var("OBS_INPUT") {
        Ok(input) => Feed::Obs {
            port: (std::env::var("OBS_PORT").ok()).map_or(Ok(obs::PORT), |port| port.parse())?,
            password: std::env::var("OBS_PASSWORD").ok(),
            input,
        },
        Err(_) => Feed::Window,
    };
    let mut capture =
        Capture::open(&feed).map_err(|status| anyhow::anyhow!("取り込めない（{status:?}）"))?;
    let (width, height) = capture.size();
    println!(
        "取り込みを始めるまで {:.0?}　大きさ {width}x{height}",
        started.elapsed()
    );

    for index in 0..count {
        let reading = Instant::now();
        // 画面が変わらないとフレームは来ない
        let image = loop {
            if let Some(mut images) = capture.read(&[[0, 0, width, height]])? {
                break images.remove(0);
            }
            anyhow::ensure!(
                reading.elapsed() < Duration::from_secs(5),
                "5 秒待ってもフレームが来ない"
            );
        };
        let read = reading.elapsed();

        let path = if count == 1 {
            out.clone()
        } else {
            let path = Path::new(&out);
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            path.with_file_name(format!("{stem}-{}.png", index + 1))
                .to_string_lossy()
                .into_owned()
        };
        image.save(&path)?;
        println!("{path}　新しい画面を待って読み出すまで {read:.1?}");
        if index + 1 < count {
            std::thread::sleep(interval.saturating_sub(reading.elapsed()));
        }
    }
    Ok(())
}
