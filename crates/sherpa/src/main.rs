//! PC の中で、確定の文字起こしを ReazonSpeech k2 で作る（DESIGN.md 9.14）。アプリが起動し、標準入出力でやりとりする。
//! ポートを開かないので、ファイアウォールの確認が出ず、ほかの PC からも頼めない。
//!
//! - 引数: k2 の encoder、decoder、joiner、tokens.txt のパス。用語があれば、5 つめにホットワードのファイル（1 行に 1 語）
//! - 入力: 1 発話ずつ、4 バイトのサンプル数（リトルエンディアン）と、16kHz mono の f32 の音声
//! - 出力: モデルを読み終えたら `{"ready":true}`、そのあと 1 発話に 1 行の `{"text":"…"}`。
//!   文字と文字の間の無音（話の間）には、空白を入れる（Chrome と同じ形。アプリが句読点にする。DESIGN.md 9.15）
//!
//! 入力が閉じたら終わる（アプリが落ちても残らない）

use std::io::{self, BufWriter, Read, Write};

use anyhow::Context;
use serde_json::json;
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

const SAMPLE_RATE: usize = 16_000;
/// 前後に足す無音。足さないと、最後の句より前が抜ける（ReazonSpeech の公式の処理と同じ長さ。DESIGN.md 3 章）
const PAD: usize = SAMPLE_RATE * 9 / 10;
/// 用語の強さ。2.0〜3.0 で用語がいちばん書け、4.0 では文字の誤りが増えた（DESIGN.md 3 章）
const HOTWORDS_SCORE: f32 = 2.0;
/// 1 発話の上限（10 分）。これより長い数は、入力が壊れている
const MAX_SAMPLES: usize = 600 * SAMPLE_RATE;
/// 音の大きさを見る区切り（20 ms）
const FRAME: usize = SAMPLE_RATE / 50;
/// この長さ以上の無音が文字の間にあれば、話の間とする（0.08〜0.16 秒で測って、適合率と再現率の釣り合いがよかった）
const PAUSE_FRAMES: usize = 6;
/// 発話のいちばん大きい音に比べて、これより小さい音を無音とする
const QUIET: f32 = 0.04;
/// 文字の出た時刻から、その文字の音が続くとみなす長さ（秒）
const CHAR_SECONDS: f32 = 0.06;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [encoder, decoder, joiner, tokens, rest @ ..] = args.as_slice() else {
        anyhow::bail!("usage: jimaku-sherpa ENCODER DECODER JOINER TOKENS [HOTWORDS]");
    };
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.transducer.encoder = Some(encoder.clone());
    config.model_config.transducer.decoder = Some(decoder.clone());
    config.model_config.transducer.joiner = Some(joiner.clone());
    config.model_config.tokens = Some(tokens.clone());
    // ゲームと同時に動くので、1 スレッドに留める（1 発話 0.1 秒ほど）
    config.model_config.num_threads = 1;
    // ホットワードは、1 文字ずつ（漢字もかなも）トークンにする
    config.model_config.modeling_unit = Some("cjkchar".into());
    // ホットワードは modified_beam_search でしか効かない
    if let Some(hotwords) = rest.first() {
        config.decoding_method = Some("modified_beam_search".into());
        config.hotwords_file = Some(hotwords.clone());
        config.hotwords_score = HOTWORDS_SCORE;
    }
    let recognizer = OfflineRecognizer::create(&config).context("load the model")?;
    emit(&json!({ "ready": true }))?;

    let mut input = io::stdin().lock();
    let mut count = [0; 4];
    while input.read_exact(&mut count).is_ok() {
        let count = u32::from_le_bytes(count) as usize;
        anyhow::ensure!(count <= MAX_SAMPLES, "too long: {count} samples");
        let mut bytes = vec![0; count * 4];
        input.read_exact(&mut bytes)?;
        let speech: Vec<f32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        let mut samples = vec![0.0; PAD];
        samples.extend(&speech);
        samples.resize(samples.len() + PAD, 0.0);
        let stream = recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE as i32, &samples);
        recognizer.decode(&stream);
        let text = match stream.get_result() {
            Some(result) => with_pauses(&result.tokens, result.timestamps.as_deref(), &speech)
                .unwrap_or(result.text),
            None => String::new(),
        };
        emit(&json!({ "text": text.trim() }))?;
    }
    Ok(())
}

/// 文字の間に話の間（`PAUSE_FRAMES` 以上の無音）があれば空白を入れた文。時刻がなければ None。
/// k2 の時刻は文字の出た時刻だけで、伸ばした音（「うわー」）でも同じくらい空くので、音の大きさで無音を確かめる
fn with_pauses(tokens: &[String], timestamps: Option<&[f32]>, speech: &[f32]) -> Option<String> {
    let times = timestamps.filter(|t| t.len() == tokens.len())?;
    let loudness: Vec<f32> = speech
        .chunks(FRAME)
        .map(|frame| (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt())
        .collect();
    let loudest = loudness.iter().copied().fold(0.0, f32::max);
    // 雑音のあるマイクでは、いちばん小さい 1 割の音（雑音）の 2 倍までを無音とする
    let mut sorted = loudness.clone();
    sorted.sort_by(f32::total_cmp);
    let noise = sorted.get(sorted.len() / 10).copied().unwrap_or(0.0);
    let threshold = (loudest * QUIET).max(noise * 2.0);
    let quiet: Vec<bool> = loudness.iter().map(|l| *l < threshold).collect();
    // 時刻は、前に足した無音のぶん後ろにずれている
    let frame_at = |seconds: f32| {
        let seconds = seconds - PAD as f32 / SAMPLE_RATE as f32;
        ((seconds.max(0.0) * SAMPLE_RATE as f32) as usize / FRAME).min(quiet.len())
    };
    let mut text = String::new();
    for (i, token) in tokens.iter().enumerate() {
        text.push_str(token);
        let Some(next) = times.get(i + 1) else {
            break;
        };
        let (from, to) = (frame_at(times[i] + CHAR_SECONDS), frame_at(*next));
        let longest = quiet[from..to.max(from)]
            .split(|q| !q)
            .map(<[bool]>::len)
            .max()
            .unwrap_or(0);
        if longest >= PAUSE_FRAMES {
            text.push(' ');
        }
    }
    Some(text)
}

fn emit(message: &serde_json::Value) -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    writeln!(out, "{message}")?;
    out.flush()
}
