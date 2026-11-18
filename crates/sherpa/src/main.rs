//! PC の中で、確定の文字起こしを ReazonSpeech k2 で作る（DESIGN.md 9.14）。アプリが起動し、標準入出力でやりとりする。
//! ポートを開かないので、ファイアウォールの確認が出ず、ほかの PC からも頼めない。
//!
//! - 引数: k2 の encoder、decoder、joiner、tokens.txt のパス。用語があれば、5 つめにホットワードのファイル（1 行に 1 語）
//! - 入力: 1 発話ずつ、4 バイトのサンプル数（リトルエンディアン）と、16kHz mono の f32 の音声
//! - 出力: モデルを読み終えたら `{"ready":true}`、そのあと 1 発話に 1 行の `{"text":"…"}`
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
        let mut samples = vec![0.0; PAD];
        samples.extend(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b)),
        );
        samples.resize(samples.len() + PAD, 0.0);
        let stream = recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE as i32, &samples);
        recognizer.decode(&stream);
        let text = stream.get_result().map(|r| r.text).unwrap_or_default();
        emit(&json!({ "text": text.trim() }))?;
    }
    Ok(())
}

fn emit(message: &serde_json::Value) -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    writeln!(out, "{message}")?;
    out.flush()
}
