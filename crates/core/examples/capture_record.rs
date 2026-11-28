//! 遊んでいる間の Deadlock の画面を、読むところだけ切り出して書き続ける（DESIGN.md 18.6 節の測定に使う）。
//! 上の端（上部バーと時計）と左下（アイテム欄）を PNG に書く。評価に足す画面は、ここから選ぶ。
//! プレイヤーのアバターなどが写るので、書いたものはそのままリポジトリに入れない。
//!
//! usage: cargo run -p overlays-core --release --example capture_record -- <出力のフォルダ> [分]
//!   `上の端` は 5 秒ごと、`左下` は 1 秒ごとに読んで、前に書いたものと違うときだけ書く。
//!   取り込みを開くたびに、画面の全体を 1 枚書く（解像度や表示の設定を変えたときの確かめ）。
//!   フォルダに `stop` というファイルを置くか、ウィンドウが 30 秒なくなると終わる

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Context;
use overlays_core::screen::BASE_HEIGHT;
use overlays_core::screen::capture::{Capture, Feed};

/// 上の端を書く間隔（秒）。左下は毎秒
const TOP_EVERY: u64 = 5;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let folder = args.next().context("出力のフォルダを渡す")?;
    let minutes: u64 = args.next().map(|v| v.parse()).transpose()?.unwrap_or(120);
    let folder = Path::new(&folder);
    std::fs::create_dir_all(folder)?;
    let stop = folder.join("stop");

    let started = Instant::now();
    let seconds = || started.elapsed().as_secs();
    let mut seen = false;
    let mut gone = Instant::now();
    while started.elapsed() < Duration::from_secs(minutes * 60) && !stop.exists() {
        let Ok(mut capture) = Capture::open(&Feed::Window) else {
            if seen && gone.elapsed() > Duration::from_secs(30) {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        seen = true;
        let (width, height) = capture.size();
        let scale = height as f32 / BASE_HEIGHT;
        println!("{:>5} 開いた {width}x{height}", seconds());
        let at = |v: f32| (v * scale).round() as u32;
        let areas = [
            [0, 0, width, at(120.0).min(height)],
            [
                0,
                at(890.0).min(height - 1),
                at(420.0).min(width),
                height - at(890.0).min(height - 1),
            ],
        ];

        let mut whole = false;
        let mut last = None;
        let mut tick = 0;
        let opened = Instant::now();
        let result = loop {
            if stop.exists() || started.elapsed() > Duration::from_secs(minutes * 60) {
                break Ok(());
            }
            if !whole {
                match capture.read(&[[0, 0, width, height]]) {
                    Ok(Some(images)) => {
                        images[0].save(folder.join(format!("{:05}-whole.png", seconds())))?;
                        whole = true;
                    }
                    Ok(None) => {}
                    Err(error) => break Err(error),
                }
            }
            match capture.read(&areas) {
                Ok(Some(mut images)) => {
                    let items = images.pop().expect("items");
                    let top = images.pop().expect("top");
                    if tick % TOP_EVERY == 0 {
                        top.save(folder.join(format!("{:05}-top.png", seconds())))?;
                    }
                    if last.as_ref() != Some(&items) {
                        items.save(folder.join(format!("{:05}-items.png", seconds())))?;
                        last = Some(items);
                    }
                }
                Ok(None) => println!("{:>5} フレームが来ない", seconds()),
                Err(error) => break Err(error),
            }
            tick += 1;
            std::thread::sleep(Duration::from_secs(tick).saturating_sub(opened.elapsed()));
        };
        if let Err(error) = result {
            println!("{:>5} 閉じた: {error:#}", seconds());
        }
        gone = Instant::now();
    }
    println!("{:>5} 終わり", seconds());
    Ok(())
}
