//! 途中の字幕と途中の翻訳を PC の中で作る（DESIGN.md 9.6）。アプリが起動し、標準入出力でやりとりする。
//!
//! - 引数: モデルを置くフォルダ、用語の表（JSON の `[["ヘイズ", "Haze"], ...]`）のパス、
//!   翻訳先（`en,ru` のようにコンマで区切る）、確定の字幕も作るなら `final`、作らないなら `partial`
//! - 入力: 1 バイトの種類と、種類が音声なら 4 バイトの長さ（リトルエンディアン）と 16kHz の f32 の音声
//! - 出力: 1 行に 1 つの JSON（`progress`、`ready`、`text`、`final`）。`final` は区切りごとに 1 つ返す
//!
//! 入力が閉じたら終わる（アプリが落ちても残らない）

mod furigana;
mod models;

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use furigana::Furigana;
use fxtranslate::engine::Engine;
use models::PairPaths;
use serde_json::{Value, json};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

const AUDIO: u8 = 1;
const CUT: u8 = 2;
const SAMPLE_RATE: usize = 16_000;
/// 長い発話でも、再デコードの時間が伸び続けないようにする
const MAX_SAMPLES: usize = 10 * SAMPLE_RATE;
/// 短い発話は滑らかに更新する。長くなったら認識の時間の 1.25 倍に広げ、CPU を占有しない
const MIN_TICK: Duration = Duration::from_millis(200);

/// 開いている発話の音声。最新だけあればいいので、デコードが追いつかなければ途中を飛ばす
#[derive(Default)]
struct Speech {
    samples: Vec<f32>,
    /// 区切るたびに増える。デコード中に区切られたら結果を捨てる
    generation: u64,
    /// 前のデコードのあとに音声が増えた
    fresh: bool,
    /// 区切られた発話。確定の字幕を作る
    finished: VecDeque<Vec<f32>>,
    closed: bool,
}

type Shared = Arc<(Mutex<Speech>, Condvar)>;

/// 字幕を作るもの。Engine はスレッドをまたげないので、メインのスレッドで持つ
struct Local {
    recognizer: OfflineRecognizer,
    furigana: Furigana,
    japanese_english: Engine,
    /// 英語から訳す言語
    from_english: Vec<(String, Engine)>,
    translate_english: bool,
    terms: Vec<(String, String)>,
}

impl Local {
    fn decode(&self, samples: &[f32]) -> String {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE as i32, samples);
        self.recognizer.decode(&stream);
        stream
            .get_result()
            .map(|r| r.text.trim().to_owned())
            .unwrap_or_default()
    }

    /// 用語を先に英語にしておくと、訳が崩れない（「ヘイズ」が Hayes にならない）
    fn translate(&self, text: &str) -> BTreeMap<String, String> {
        let replaced = self
            .terms
            .iter()
            .fold(text.to_owned(), |text, (source, target)| {
                text.replace(source, target)
            });
        let english = self.japanese_english.translate(&replaced);
        let mut translations: BTreeMap<String, String> = self
            .from_english
            .iter()
            .map(|(lang, engine)| (lang.clone(), engine.translate(&english)))
            .collect();
        if self.translate_english {
            translations.insert("en".to_owned(), english);
        }
        translations
    }

    fn message(&self, kind: &str, text: &str) -> Value {
        json!({
            "type": kind,
            "text": text,
            "parts": self.furigana.annotate(text),
            "translations": self.translate(text),
        })
    }
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().context("models folder")?);
    let terms = load_terms(&args.next().context("terms file")?);
    let targets: Vec<String> = args
        .next()
        .context("targets")?
        .split(',')
        .filter(|t| !t.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    let finals = args.next().context("mode")? == "final";

    let others: Vec<String> = targets.iter().filter(|t| *t != "en").cloned().collect();
    let paths = models::ensure(&dir, &others, |loaded, total| {
        emit(&json!({ "type": "progress", "loaded": loaded, "total": total }));
    })?;
    let mut pairs = paths.pairs.iter();
    let (_, japanese_english) = pairs.next().context("ja-en model")?;
    let local = Local {
        recognizer: recognizer(&paths)?,
        furigana: Furigana::new()?,
        japanese_english: engine(japanese_english)?,
        from_english: pairs
            .map(|(name, pair)| {
                let lang = name.trim_start_matches("en-").to_owned();
                Ok((lang, engine(pair)?))
            })
            .collect::<anyhow::Result<_>>()?,
        translate_english: targets.iter().any(|t| t == "en"),
        terms,
    };
    emit(&json!({ "type": "ready" }));

    let speech: Shared = Arc::default();
    let reader = speech.clone();
    std::thread::spawn(move || {
        if let Err(e) = read_input(&reader, finals) {
            eprintln!("input: {e}");
        }
        reader.0.lock().unwrap().closed = true;
        reader.1.notify_all();
    });

    let mut shown = String::new();
    let mut shown_generation = 0;
    loop {
        let (samples, generation, finished) = {
            let (lock, wake) = &*speech;
            let mut s = wake
                .wait_while(lock.lock().unwrap(), |s| {
                    !s.fresh && !s.closed && s.finished.is_empty()
                })
                .unwrap();
            if s.closed && s.finished.is_empty() {
                return Ok(());
            }
            // 確定の字幕を先に作る
            if let Some(finished) = s.finished.pop_front() {
                (Vec::new(), s.generation, Some(finished))
            } else {
                s.fresh = false;
                let start = s.samples.len().saturating_sub(MAX_SAMPLES);
                (s.samples[start..].to_vec(), s.generation, None)
            }
        };
        if let Some(finished) = finished {
            let text = local.decode(&finished);
            emit(&local.message("final", &text));
            continue;
        }
        let started = Instant::now();
        let text = local.decode(&samples);
        if generation != shown_generation {
            shown.clear();
            shown_generation = generation;
        }
        if !text.is_empty() && text != shown && speech.0.lock().unwrap().generation == generation {
            emit(&local.message("text", &text));
            shown = text;
        }
        let elapsed = started.elapsed();
        std::thread::sleep(MIN_TICK.max(elapsed.mul_f32(1.25)).saturating_sub(elapsed));
    }
}

fn recognizer(paths: &models::Paths) -> anyhow::Result<OfflineRecognizer> {
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice.model = Some(path(&paths.speech_model));
    config.model_config.sense_voice.language = Some("ja".to_owned());
    config.model_config.sense_voice.use_itn = false;
    config.model_config.tokens = Some(path(&paths.tokens));
    // ゲームと同時に動くので、1 スレッドに留める
    config.model_config.num_threads = 1;
    OfflineRecognizer::create(&config).context("load the speech model")
}

fn engine(pair: &PairPaths) -> anyhow::Result<Engine> {
    let shortlist = std::fs::read(&pair.shortlist)?;
    let engine = Engine::load_mmapped(
        &pair.model,
        &pair.source_vocabulary,
        &pair.target_vocabulary,
    )
    .map_err(|e| anyhow::anyhow!("load the translation model: {e:?}"))?;
    Ok(engine.with_shortlist_bytes(&shortlist))
}

fn path(path: &Path) -> String {
    path.display().to_string()
}

fn read_input(speech: &Shared, finals: bool) -> io::Result<()> {
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
                let samples = std::mem::take(&mut s.samples);
                if finals {
                    s.finished.push_back(samples);
                }
                s.generation += 1;
                s.fresh = false;
                speech.1.notify_all();
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

fn emit(message: &Value) {
    let mut out = BufWriter::new(io::stdout().lock());
    let _ = writeln!(out, "{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longer_terms_come_first() {
        let path = std::env::temp_dir().join("jimaku-local-terms.json");
        std::fs::write(
            &path,
            r#"[["カース", "Curse"], ["カースブレイカー", "Breaker"]]"#,
        )
        .unwrap();
        let terms = load_terms(path.to_str().unwrap());
        assert_eq!(terms[0].1, "Breaker");
    }
}
