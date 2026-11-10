//! 用語の判定と翻訳を、モデルごとにアプリと同じ問い合わせ（`Enricher::terms_and_translations`）で比べる
//! （DESIGN.md 9.3）。文は `experiments/results/entity_llm.json` の 37 文で、正解は GPT-6.1 Terra が
//! 3 回とも同じに答え、確かめた用語の区切り（DESIGN.md 9.1）。
//!
//! usage: OPENAI_API_KEY=… ANTHROPIC_API_KEY=… cargo run -p overlays-core --example terms_eval -- MODEL... [--repeat N]
//!   出力: experiments/results/terms_eval.json（モデルごとに足す。同じモデルは置き換える）
//!   辞書はアプリの保存した分（`{データディレクトリ}/assets`）を使う。
//!   `claude-` で始まるモデルは Anthropic の Messages API に、同じ問い合わせ（`Enricher::question`）を
//!   構造化出力で送る。`qwen` で始まるモデルは QwenCloud（シンガポール）の OpenAI 互換の口に、
//!   同じ問い合わせを Chat Completions の構造化出力で送る（`QWEN_SOUTH_API_KEY`）。
//!   どちらも推論は止める（OpenAI の `reasoning_effort: none` に合わせる）

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use overlays_core::access::Access;
use overlays_core::assets;
use overlays_core::enrich::{Enricher, Question, Span};
use overlays_core::settings::Models;
use overlays_core::types::{Glossary, Token};
use overlays_core::usage::Meter;
use serde_json::{Value, json};

const CASES: &str = "experiments/results/entity_llm.json";
const OUT: &str = "experiments/results/terms_eval.json";
/// 翻訳先。ロシア語は語尾が変わり、名前が崩れやすい（DESIGN.md 9.3）
const TARGETS: [&str; 2] = ["en", "ru"];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut models = Vec::new();
    let mut repeat = 3;
    let mut cases_path = CASES.to_owned();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repeat" => repeat = args.next().context("--repeat N")?.parse()?,
            // 別の文と正解で測る（`experiments/terms_cases.py` の出力）。結果は文の名前を付けた別のファイルに書く
            "--cases" => cases_path = args.next().context("--cases PATH")?,
            model => models.push(model.to_owned()),
        }
    }
    anyhow::ensure!(
        !models.is_empty(),
        "usage: terms_eval MODEL... [--repeat N]"
    );
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    let qwen_key = std::env::var("QWEN_SOUTH_API_KEY").unwrap_or_default();
    let http = reqwest::Client::new();

    let cases: Vec<Value> = serde_json::from_slice(&std::fs::read(&cases_path)?)?;
    let out = match cases_path.as_str() {
        CASES => OUT.to_owned(),
        path => {
            let stem = Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .context("--cases PATH")?;
            OUT.replace(".json", &format!(".{stem}.json"))
        }
    };
    let languages: Vec<String> = ["ja"].into_iter().chain(TARGETS).map(Into::into).collect();
    let loaded = assets::load(
        &overlays_core::default_data_dir().join("assets"),
        &languages,
    )
    .await?;
    let glossary = loaded.glossary;

    let mut report: BTreeMap<String, Value> = std::fs::read(&out)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    for model in &models {
        let meter = Arc::new(Meter::new(
            std::env::temp_dir().join("terms_eval_usage.json"),
        ));
        let enricher = Enricher::new(
            Access::key(overlays_core::secrets::Provider::OpenAi, key.clone()),
            &Models {
                translate: model.clone(),
                ..Models::default()
            },
            "ja".into(),
            TARGETS.map(Into::into).to_vec(),
            true,
            meter.clone(),
        );
        let other = model.starts_with("claude-") || model.starts_with("qwen");
        let mut runs = Vec::new();
        let (mut correct, mut leftovers, mut stray, mut raw, mut failed, mut total_ms) =
            (0, 0, 0, 0, 0, 0u128);
        let mut other_usd = 0.0;
        for case in &cases {
            let text = case["text"].as_str().context("text")?;
            let context: Vec<String> = serde_json::from_value(case["context"].clone())?;
            let expected = expected_terms(case);
            for _ in 0..repeat {
                let started = Instant::now();
                let answer = if other {
                    let question = enricher
                        .question(text, &context, Some(&glossary))
                        .context("nothing to ask")?;
                    let asked = if model.starts_with("claude-") {
                        ask_claude(&http, &anthropic_key, model, question).await
                    } else {
                        ask_qwen(&http, &qwen_key, model, question).await
                    };
                    asked.and_then(|(answer, usd)| {
                        other_usd += usd;
                        enricher.read(text, Some(&glossary), answer).map(Some)
                    })
                } else {
                    enricher
                        .terms_and_translations(text, &context, Some(&glossary), |_| {})
                        .await
                };
                let ms = started.elapsed().as_millis();
                total_ms += ms;
                let Ok(Some((spans, translations))) = answer else {
                    failed += 1;
                    runs.push(json!({ "text": text, "ms": ms, "error": format!("{answer:?}") }));
                    continue;
                };
                let terms = found_terms(&spans);
                let ok = terms == expected;
                correct += usize::from(ok);
                let mut translated = BTreeMap::new();
                for (lang, tokens) in &translations {
                    let written = written(tokens, &glossary);
                    // 訳文に日本語が残った（DESIGN.md 9.3 の「実況」）
                    if has_japanese(&written) {
                        leftovers += 1;
                    }
                    // 参照を `⟦…⟧` で囲まずに書いた（`[hero:67]` などが字幕にそのまま出る）
                    if has_raw_reference(&written) {
                        raw += 1;
                    }
                    // 原文で見つけていない用語は訳文に出ない（`resolve`）。区切りに崩れがあると用語が抜ける
                    let refs: Vec<&str> = tokens
                        .iter()
                        .filter_map(|t| match t {
                            Token::Entity { reference, .. } => Some(reference.as_str()),
                            Token::Text { .. } => None,
                        })
                        .collect();
                    if refs.iter().any(|r| !terms.iter().any(|(_, t)| t == r)) {
                        stray += 1;
                    }
                    translated.insert(lang.clone(), written);
                }
                runs.push(json!({
                    "text": text,
                    "ms": ms,
                    "ok": ok,
                    "terms": terms,
                    "expected": expected,
                    "translations": translated,
                }));
            }
        }
        let n = cases.len() * repeat;
        let usage = if other {
            json!({ "translate": { "usd": other_usd } })
        } else {
            serde_json::to_value(meter.watch().borrow().session.clone())?
        };
        let summary = json!({
            "runs": n,
            "termsCorrect": correct,
            "failed": failed,
            "japaneseLeftInTranslation": leftovers,
            "strayTermsInTranslation": stray,
            "rawReferencesInTranslation": raw,
            "averageMs": total_ms / n as u128,
            "usage": usage,
        });
        println!("{model}: {summary}");
        report.insert(model.clone(), json!({ "summary": summary, "runs": runs }));
    }
    std::fs::write(Path::new(&out), serde_json::to_vec_pretty(&report)?)?;
    println!("wrote {out}");
    Ok(())
}

/// Claude の Messages API に、アプリと同じ問い合わせを送る。答えの JSON と、料金（ドル）を返す。
/// 単価は Claude Haiku 5.5（10 万トークンまで。キャッシュの読み 0.1 倍、5 分の書き込み 1.25 倍）
async fn ask_claude(
    http: &reqwest::Client,
    key: &str,
    model: &str,
    question: Question,
) -> anyhow::Result<(Value, f64)> {
    anyhow::ensure!(model == "claude-haiku-5-5", "unpriced model: {model}");
    let body = json!({
        "model": model,
        "max_tokens": 4096,
        // 辞書の一覧は毎回同じなので、キャッシュから読む
        "system": [{ "type": "text", "text": &*question.system, "cache_control": { "type": "ephemeral" } }],
        "messages": [{ "role": "user", "content": question.user }],
        "thinking": { "type": "disabled" },
        "output_config": {
            "effort": "low",
            "format": { "type": "json_schema", "schema": question.schema },
        },
    });
    let res = http
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .timeout(std::time::Duration::from_secs(8))
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    let res: Value = res.json().await?;
    anyhow::ensure!(status.is_success(), "{status}: {res}");
    anyhow::ensure!(
        res["stop_reason"] == "end_turn",
        "stopped: {}",
        res["stop_reason"]
    );
    let text = res["content"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|b| b["type"] == "text")
        .and_then(|b| b["text"].as_str())
        .context("no text")?;
    let u = &res["usage"];
    let n = |k: &str| u[k].as_f64().unwrap_or(0.0);
    let usd = (n("input_tokens") * 0.10
        + n("cache_read_input_tokens") * 0.01
        + n("cache_creation_input_tokens") * 0.125
        + n("output_tokens") * 0.50)
        / 1e6;
    Ok((serde_json::from_str(text)?, usd))
}

/// QwenCloud（シンガポール）の Chat Completions に、アプリと同じ問い合わせを送る。答えの JSON と、料金（ドル）を返す。
/// 単価はシンガポールの 100 万トークンまでの段。辞書の一覧は明示のキャッシュに置く（読みは入力の 0.1 倍、書き込みは 1.25 倍）
async fn ask_qwen(
    http: &reqwest::Client,
    key: &str,
    model: &str,
    question: Question,
) -> anyhow::Result<(Value, f64)> {
    let (input, output) = match model {
        "qwen3.8-flash" => (0.15, 0.47),
        "qwen3.8-max" => (2.0, 6.0),
        _ => anyhow::bail!("unpriced model: {model}"),
    };
    let body = json!({
        "model": model,
        "messages": [
            // 辞書の一覧は毎回同じなので、キャッシュから読む
            { "role": "system", "content": [
                { "type": "text", "text": &*question.system, "cache_control": { "type": "ephemeral" } },
            ] },
            { "role": "user", "content": question.user },
        ],
        "enable_thinking": false,
        "response_format": {
            "type": "json_schema",
            "json_schema": { "name": "analysis", "strict": true, "schema": question.schema },
        },
    });
    let res = http
        .post("https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1/chat/completions")
        .bearer_auth(key)
        .timeout(std::time::Duration::from_secs(8))
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    let res: Value = res.json().await?;
    anyhow::ensure!(status.is_success(), "{status}: {res}");
    let choice = &res["choices"][0];
    anyhow::ensure!(
        choice["finish_reason"] == "stop",
        "stopped: {}",
        choice["finish_reason"]
    );
    let text = choice["message"]["content"].as_str().context("no text")?;
    let u = &res["usage"];
    let n = |v: &Value| v.as_f64().unwrap_or(0.0);
    let details = &u["prompt_tokens_details"];
    let (read, written) = (
        n(&details["cached_tokens"]),
        n(&details["cache_creation_input_tokens"]),
    );
    let fresh = n(&u["prompt_tokens"]) - read - written;
    let usd = (fresh * input
        + read * input * 0.1
        + written * input * 1.25
        + n(&u["completion_tokens"]) * output)
        / 1e6;
    Ok((serde_json::from_str(text)?, usd))
}

/// 正解の用語（話した言葉と参照）。Terra の 3 回の答えは同じなので、最初の答えを使う
fn expected_terms(case: &Value) -> Vec<(String, String)> {
    case["llm"][0]["answer"]["segments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            Some((
                s["text"].as_str()?.to_owned(),
                s["ref"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

fn found_terms(spans: &[Span]) -> Vec<(String, String)> {
    spans
        .iter()
        .filter_map(|s| match s {
            Span::Entity { reference, spoken } => Some((spoken.clone(), reference.clone())),
            Span::Text(_) => None,
        })
        .collect()
}

/// 訳文を読める形にする。用語はその言語の公式名（なければ英語名）で書く
fn written(tokens: &[Token], glossary: &Glossary) -> String {
    tokens
        .iter()
        .map(|t| match t {
            Token::Text { text, .. } => text.clone(),
            Token::Entity { reference, .. } => glossary
                .entities
                .get(reference)
                .and_then(|e| e.names.get("en"))
                .map_or_else(|| format!("⟦{reference}⟧"), |n| format!("⟦{n}⟧")),
        })
        .collect()
}

fn has_raw_reference(text: &str) -> bool {
    ["hero:", "ability:", "item:"]
        .iter()
        .any(|kind| text.contains(kind))
}

fn has_japanese(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c, '\u{3040}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}'))
}
