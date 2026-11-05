//! 途中の字幕と途中の翻訳を PC の中で作る（DESIGN.md 9.6）。アプリが起動し、標準入出力でやりとりする。
//!
//! - 引数: モデルを置くフォルダ、話す言語、用語の表（JSON の `[["ヘイズ", "Haze"], ...]`）のパス
//! - 入力: 1 バイトの種類と、種類が音声なら 4 バイトの長さ（リトルエンディアン）と 16kHz の f32 の音声
//! - 出力: 1 行に 1 つの JSON（`progress`、`ready`、`text`）
//!
//! 入力が閉じたら終わる（アプリが落ちても残らない）

mod models;

use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use fxtranslate::engine::Engine;
use serde_json::{Value, json};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

const AUDIO: u8 = 1;
const CUT: u8 = 2;
const SAMPLE_RATE: usize = 16_000;
/// 長い発話でも、再デコードの時間が伸び続けないようにする
const MAX_SAMPLES: usize = 10 * SAMPLE_RATE;
const MIN_TICK: Duration = Duration::from_millis(300);

/// 開いている発話の音声。最新だけあればいいので、デコードが追いつかなければ途中を飛ばす
#[derive(Default)]
struct Speech {
    samples: Vec<f32>,
    /// 区切るたびに増える。デコード中に区切られたら結果を捨てる
    generation: u64,
    /// 前のデコードのあとに音声が増えた
    fresh: bool,
    closed: bool,
}

type Shared = Arc<(Mutex<Speech>, Condvar)>;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().context("models folder")?);
    let lang = args.next().context("language")?;
    let terms = load_terms(&args.next().context("terms file")?);

    let paths = models::ensure(&dir, |loaded, total| {
        emit(&json!({ "type": "progress", "loaded": loaded, "total": total }));
    })?;
    let recognizer = recognizer(&paths, &lang)?;
    // 翻訳のモデルは日本語から英語だけ。Engine はスレッドをまたげないので、ここで持つ
    let translator = (lang == "ja")
        .then(|| load_translator(&paths))
        .transpose()?;
    emit(&json!({ "type": "ready" }));

    let speech: Shared = Arc::default();
    let reader = speech.clone();
    std::thread::spawn(move || {
        if let Err(e) = read_input(&reader) {
            eprintln!("input: {e}");
        }
        reader.0.lock().unwrap().closed = true;
        reader.1.notify_all();
    });

    let mut shown = String::new();
    let mut shown_generation = 0;
    loop {
        let (samples, generation) = {
            let (lock, wake) = &*speech;
            let mut s = wake
                .wait_while(lock.lock().unwrap(), |s| !s.fresh && !s.closed)
                .unwrap();
            if s.closed {
                return Ok(());
            }
            s.fresh = false;
            let start = s.samples.len().saturating_sub(MAX_SAMPLES);
            (s.samples[start..].to_vec(), s.generation)
        };
        let started = Instant::now();
        let text = decode(&recognizer, &samples);
        if generation != shown_generation {
            shown.clear();
            shown_generation = generation;
        }
        if !text.is_empty() && text != shown && speech.0.lock().unwrap().generation == generation {
            let translation = translator
                .as_ref()
                .map(|t| t.translate(&replace_terms(&text, &terms)));
            emit(&json!({ "type": "text", "text": text, "translation": translation }));
            shown = text;
        }
        // 長い発話では、デコードが CPU を占め続けないよう間を空ける
        std::thread::sleep(
            MIN_TICK
                .max(started.elapsed() * 2)
                .saturating_sub(started.elapsed()),
        );
    }
}

fn recognizer(paths: &models::Paths, lang: &str) -> anyhow::Result<OfflineRecognizer> {
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice.model = Some(path(&paths.speech_model));
    config.model_config.sense_voice.language = Some(lang.to_owned());
    config.model_config.sense_voice.use_itn = false;
    config.model_config.tokens = Some(path(&paths.tokens));
    // ゲームと同時に動くので、1 スレッドに留める
    config.model_config.num_threads = 1;
    OfflineRecognizer::create(&config).context("load the speech model")
}

fn load_translator(paths: &models::Paths) -> anyhow::Result<Engine> {
    let shortlist = std::fs::read(&paths.shortlist)?;
    let engine = Engine::load_mmapped(&paths.translation, &paths.vocabulary, &paths.vocabulary)
        .map_err(|e| anyhow::anyhow!("load the translation model: {e:?}"))?;
    Ok(engine.with_shortlist_bytes(&shortlist))
}

fn path(path: &Path) -> String {
    path.display().to_string()
}

fn decode(recognizer: &OfflineRecognizer, samples: &[f32]) -> String {
    let stream = recognizer.create_stream();
    stream.accept_waveform(SAMPLE_RATE as i32, samples);
    recognizer.decode(&stream);
    stream
        .get_result()
        .map(|r| r.text.trim().to_owned())
        .unwrap_or_default()
}

fn read_input(speech: &Shared) -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut kind = [0];
    while input.read_exact(&mut kind).is_ok() {
        match kind[0] {
            AUDIO => {
                let mut len = [0; 4];
                input.read_exact(&mut len)?;
                let mut bytes = vec![0; u32::from_le_bytes(len) as usize];
                input.read_exact(&mut bytes)?;
                let mut s = speech.0.lock().unwrap();
                s.samples.extend(
                    bytes
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                );
                s.fresh = true;
                speech.1.notify_all();
            }
            CUT => {
                let mut s = speech.0.lock().unwrap();
                s.samples.clear();
                s.generation += 1;
                s.fresh = false;
            }
            other => return Err(io::Error::other(format!("unknown input {other}"))),
        }
    }
    Ok(())
}

/// 長い名前から置き換えられるよう、長い順に並べる
fn load_terms(path: &str) -> Vec<(String, String)> {
    let mut terms: Vec<(String, String)> = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    terms.sort_by_key(|(source, _)| std::cmp::Reverse(source.chars().count()));
    terms
}

/// 用語を先に英語にしておくと、訳が崩れない（「ヘイズ」が Hayes にならない）
fn replace_terms(text: &str, terms: &[(String, String)]) -> String {
    terms
        .iter()
        .fold(text.to_owned(), |text, (source, target)| {
            text.replace(source, target)
        })
}

fn emit(message: &Value) {
    let mut out = BufWriter::new(io::stdout().lock());
    let _ = writeln!(out, "{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longer_terms_win() {
        let dir = std::env::temp_dir().join("jimaku-local-terms.json");
        std::fs::write(
            &dir,
            r#"[["カース", "Curse"], ["カースブレイカー", "Breaker"]]"#,
        )
        .unwrap();
        let terms = load_terms(dir.to_str().unwrap());
        assert_eq!(
            replace_terms("カースブレイカーとカース", &terms),
            "BreakerとCurse"
        );
    }
}
