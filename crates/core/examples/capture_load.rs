//! Deadlock のウィンドウを取り込み続けたときの負荷を測る（DESIGN.md 18.6 節）。
//! 本体の取り込み（`screen/capture.rs`。Windows だけ）で決まった秒数だけ読み、
//! 読むのに掛かった時間と、このプロセスが使った CPU の時間を出す。
//! ゲームの FPS は外から測れないので、終わりに画面の右上（ゲームが出す FPS の表示）を PNG に書く。
//! ゲームと OBS の CPU と GPU は、`experiments/capture_load.ps1` がこの例を動かしながら測る。
//!
//! usage: cargo run -p overlays-core --release --example capture_load -- <測り方> [秒数] [出力のフォルダ] [1 秒の回数]
//!   測り方: none（取り込まない）、idle（取り込むが読み出さない）、full（画面全体を読み出す）、
//!           region（上部バーとアイテム欄のあたりだけ読み出す）

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Context;
use overlays_core::screen::BASE_HEIGHT;
use overlays_core::screen::capture::Capture;

/// 読み出す範囲（1080p。左、上、幅、高さ）。上部バーと、アイテム欄（ショップを開くと上にずれる分も入れる）
const REGIONS: [[f32; 4]; 2] = [[0.0, 0.0, 1920.0, 120.0], [0.0, 890.0, 420.0, 190.0]];
/// ゲームが FPS を出す場所（1080p。右の端からの幅と、上の端からの高さ）
const FPS: (f32, f32) = (240.0, 48.0);

/// このプロセスが使った CPU の時間
#[cfg(windows)]
fn cpu_time() -> Duration {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    unsafe {
        let _ = GetProcessTimes(GetCurrentProcess(), created, exited, kernel, user);
    }
    let ticks = |time: FILETIME| (time.dwHighDateTime as u64) << 32 | time.dwLowDateTime as u64;
    Duration::from_nanos((ticks(times[2]) + ticks(times[3])) * 100)
}

#[cfg(not(windows))]
fn cpu_time() -> Duration {
    Duration::ZERO
}

fn open() -> anyhow::Result<Capture> {
    Capture::open()?.context("Deadlock のウィンドウがない（取り込めるのは Windows だけ）")
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = args
        .next()
        .context("測り方（none、idle、full、region）を渡す")?;
    anyhow::ensure!(
        ["none", "idle", "full", "region"].contains(&mode.as_str()),
        "測り方は none、idle、full、region のどれか"
    );
    let seconds: u64 = args.next().map(|v| v.parse()).transpose()?.unwrap_or(30);
    let folder = args.next().unwrap_or_else(|| ".".to_owned());
    let rate: u32 = args.next().map(|v| v.parse()).transpose()?.unwrap_or(3);
    let period = Duration::from_secs(1) / rate;

    let before = cpu_time();
    let started = Instant::now();
    let mut capture = (mode != "none").then(open).transpose()?;
    let mut reads = Vec::new();
    let mut missed = 0;
    if let Some(capture) = &mut capture {
        let (width, height) = capture.size();
        let scale = height as f32 / BASE_HEIGHT;
        let areas: Vec<[u32; 4]> = match mode.as_str() {
            "full" => vec![[0, 0, width, height]],
            "region" => REGIONS
                .iter()
                .map(|area| {
                    let [left, top, wide, high] = area.map(|v| (v * scale).round() as u32);
                    [left, top, wide.min(width - left), high.min(height - top)]
                })
                .collect(),
            // 範囲を渡さなければ、新しい 1 枚を待つだけで読み出さない
            _ => Vec::new(),
        };
        let mut tick = 0;
        while started.elapsed() < Duration::from_secs(seconds) {
            let reading = Instant::now();
            match capture.read(&areas)? {
                Some(images) => {
                    std::hint::black_box(images);
                    reads.push(reading.elapsed());
                }
                None => missed += 1,
            }
            tick += 1;
            std::thread::sleep((period * tick).saturating_sub(started.elapsed()));
        }
    } else {
        std::thread::sleep(Duration::from_secs(seconds));
    }
    let elapsed = started.elapsed();
    let cpu = cpu_time() - before;

    // FPS の表示を書く。取り込まない測り方では、ここで初めて取り込む
    let mut capture = match capture {
        Some(capture) => capture,
        None => open()?,
    };
    let (width, height) = capture.size();
    let scale = height as f32 / BASE_HEIGHT;
    let (wide, high) = (
        (FPS.0 * scale).round() as u32,
        (FPS.1 * scale).round() as u32,
    );
    let waiting = Instant::now();
    let shown = loop {
        if let Some(mut images) = capture.read(&[[width - wide, 0, wide, high]])? {
            break images.remove(0);
        }
        anyhow::ensure!(
            waiting.elapsed() < Duration::from_secs(2),
            "フレームが来ない"
        );
    };
    let shown = image::imageops::resize(
        &shown,
        wide * 3,
        high * 3,
        image::imageops::FilterType::Nearest,
    );
    let path = Path::new(&folder).join(format!("{mode}-fps.png"));
    shown.save(&path)?;

    println!(
        "{mode}　{:.1} 秒　{} 回（来なかった {missed} 回）　CPU {:.0?}（1 コアの {:.2}%）",
        elapsed.as_secs_f32(),
        reads.len(),
        cpu,
        cpu.as_secs_f32() / elapsed.as_secs_f32() * 100.0,
    );
    if !reads.is_empty() {
        reads.sort();
        let mean = reads.iter().sum::<Duration>() / reads.len() as u32;
        let at = |part: f32| reads[((reads.len() as f32 * part) as usize).min(reads.len() - 1)];
        // 新しい画面を待つ時間（平均 10 ミリ秒ほど）を含む。読み出しだけの時間は、idle との差
        println!(
            "  新しい画面を待って読み出すまで 平均 {:.2?}　95% {:.2?}　最大 {:.2?}",
            mean,
            at(0.95),
            at(1.0)
        );
    }
    println!("  FPS の表示 {}", path.display());
    Ok(())
}
