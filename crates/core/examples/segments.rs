//! アプリの発話の区切り（`segment::Segmenter`）を録音にかけて、区切りの位置を出す（DESIGN.md 3.5）。
//!
//! usage: cargo run -p overlays-core --example segments -- AUDIO.wav MIN_SILENCE_MS...
//!   出力: 無音の長さごとの、発話の始まりと終わり（ミリ秒）の JSON

use overlays_core::segment::{Event, Segmenter};
use overlays_core::settings::Segmentation;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let wav = args.next().expect("AUDIO.wav");
    let samples: Vec<f32> = hound::WavReader::open(wav)?
        .samples::<i16>()
        .map(|s| s.map(|s| f32::from(s) / f32::from(i16::MAX)))
        .collect::<Result<_, _>>()?;
    let mut out = serde_json::Map::new();
    for min_silence_ms in args {
        let mut segmenter = Segmenter::new(&Segmentation {
            min_silence_ms: min_silence_ms.parse()?,
            ..Segmentation::default()
        });
        let mut closes: Vec<_> = segmenter
            .feed(&samples)
            .into_iter()
            .filter_map(|event| match event {
                Event::Close(close) => Some(close),
                Event::Open => None,
            })
            .collect();
        closes.extend(segmenter.finish());
        let ms = |samples: u64| samples / 16;
        let spans: Vec<[u64; 2]> = closes.iter().map(|c| [ms(c.start), ms(c.end)]).collect();
        out.insert(min_silence_ms, serde_json::json!(spans));
    }
    println!("{}", serde_json::Value::Object(out));
    Ok(())
}
