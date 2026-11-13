//! 確定の文字起こしが、用語を正しく書けるかを測る（DESIGN.md 3 章）。音声は `experiments/tts_generate.py` が
//! 台本から作った合成音声で、読ませた文がそのまま正解になる。アプリと同じ部品（`stt::transcribe`）に、
//! アプリと同じヒント（言語、録音の説明、辞書の公式名）を渡す。
//!
//! usage: cargo run --release -p overlays-core --example stt_terms_eval -- SCRIPT.json TTS_DIR GLOSSARY.json OUT.json ENGINE...
//!   ENGINE は設定と同じ名前（gptTranscribe、grokVoiceTranscribe、qwenAudio、maiTranscribe2、haasJa）。
//!   鍵は OPENAI_API_KEY、XAI_API_KEY、QWEN_SOUTH_API_KEY（シンガポールの鍵）、OPENROUTER_API_KEY。
//!   断られたら（回数の上限など）、2 秒待って 3 回までやり直す。haasJa は `STT_EVAL_SPEECH`（speech.cpp と
//!   モデルを置くフォルダ。なければ取る）
//!   出力: OUT.json（エンジンごとに足す。同じものは置き換える）

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use overlays_core::access::Access;
use overlays_core::local::Speech;
use overlays_core::secrets::Provider;
use overlays_core::settings::{Engines, FinalSource, Settings};
use overlays_core::stt::{self, BatchRequest, Hints, Transcriber};
use overlays_core::subtitles::{PROMPT, keywords};
use overlays_core::types::Glossary;
use overlays_core::usage::Meter;
use serde_json::{Value, json};

/// 比べる前に、句読点・空白・記号を落とす（書き方の揺れを数えない）
fn normalize(text: &str) -> Vec<char> {
    text.chars().filter(|c| c.is_alphanumeric()).collect()
}

fn distance(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let cost = previous + usize::from(x != y);
            previous = row[j + 1];
            row[j + 1] = cost.min(previous + 1).min(row[j] + 1);
        }
    }
    row[b.len()]
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut args = std::env::args().skip(1);
    let mut path = |name: &str| {
        args.next()
            .map(PathBuf::from)
            .with_context(|| name.to_owned())
    };
    let (script, audio, glossary, out) = (
        path("SCRIPT.json")?,
        path("TTS_DIR")?,
        path("GLOSSARY.json")?,
        path("OUT.json")?,
    );
    let script: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(script)?)?;
    let glossary: Glossary = serde_json::from_str(&std::fs::read_to_string(glossary)?)?;
    let hints = Hints {
        language: "ja".into(),
        prompt: PROMPT.into(),
        keywords: keywords(&glossary, "ja"),
        heroes: overlays_core::subtitles::heroes(&glossary, "ja"),
    };
    let mut voices: Vec<PathBuf> = std::fs::read_dir(&audio)?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.is_dir())
        .collect();
    voices.sort();
    let meter = Meter::new(std::env::temp_dir().join("overlays-stt-terms-eval-usage.json"));
    let http = overlays_core::http();
    let mut results: Value = std::fs::read_to_string(&out)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}));
    for engine in args {
        let source: FinalSource = serde_json::from_value(json!(engine))?;
        let engines = Engines {
            transcription: source,
            ..Engines::CHROME
        };
        let settings = Settings {
            engines,
            ..Settings::default()
        };
        let model = settings.models_in_use().batch;
        let transcriber = match source.provider() {
            Some(provider) => {
                // Qwen の音声認識はシンガポールの鍵でしか使えない
                let name = match provider {
                    Provider::Qwen => "QWEN_SOUTH_API_KEY".to_owned(),
                    _ => format!("{}_API_KEY", provider.info().id.to_uppercase()),
                };
                let key = std::env::var(&name).with_context(|| name)?;
                Transcriber::Api(Access::key(provider, key))
            }
            None => {
                let dir = std::env::var("STT_EVAL_SPEECH").context("STT_EVAL_SPEECH")?;
                Transcriber::Local(Arc::new(Speech::start(Path::new(&dir)).await?))
            }
        };
        let (mut hit, mut terms, mut edits, mut length, mut failed) = (0, 0, 0, 0, 0);
        let mut times = Vec::new();
        let mut runs = Vec::new();
        for voice in &voices {
            for (i, line) in script.iter().enumerate() {
                let wav = std::fs::read(voice.join(format!("{i:03}.wav")))?;
                let seconds = (wav.len().saturating_sub(44)) as f64 / 32000.0;
                let mut answer = None;
                for attempt in 0..3 {
                    if attempt > 0 {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    let started = Instant::now();
                    let request = BatchRequest {
                        transcriber: &transcriber,
                        model: &model,
                        hints: &hints,
                        wav: wav.clone(),
                        seconds,
                        meter: &meter,
                    };
                    match stt::transcribe(http, request).await {
                        Ok(text) => {
                            times.push(started.elapsed().as_millis());
                            answer = Some(text);
                            break;
                        }
                        Err(e) => eprintln!("{engine} {} {i}: {e:#}", voice.display()),
                    }
                }
                failed += usize::from(answer.is_none());
                let heard = answer.unwrap_or_default();
                let (reference, written) = (
                    normalize(line["text"].as_str().context("text")?),
                    normalize(&heard),
                );
                let written_text: String = written.iter().collect();
                let missed: Vec<&str> = line["terms"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .inspect(|_| terms += 1)
                    .filter(|term| {
                        // 同じ言葉の別の書き方（「おひるね」と「お昼寝」）も、書けたと数える
                        let accepted = line["accept"][term].as_array().into_iter().flatten();
                        let spellings = accepted.filter_map(Value::as_str).chain([*term]);
                        let found = |spelling: &str| {
                            written_text.contains(&normalize(spelling).iter().collect::<String>())
                        };
                        !spellings.into_iter().any(found)
                    })
                    .collect();
                hit += line["terms"].as_array().map_or(0, Vec::len) - missed.len();
                edits += distance(&reference, &written);
                length += reference.len();
                runs.push(json!({
                    "voice": voice.file_name().and_then(|name| name.to_str()),
                    "line": i,
                    "heard": heard,
                    "missed": missed,
                }));
            }
        }
        times.sort_unstable();
        let summary = json!({
            "model": model,
            "terms": terms,
            "termsWritten": hit,
            "characters": length,
            "characterErrors": edits,
            "failed": failed,
            "medianMs": times[times.len() / 2],
            "p90Ms": times[times.len() * 9 / 10],
        });
        println!("{engine}: {summary}");
        results[&engine] = json!({ "summary": summary, "runs": runs });
        std::fs::write(&out, serde_json::to_string_pretty(&results)?)?;
    }
    println!("{:?}", meter.watch().borrow().session);
    Ok(())
}
