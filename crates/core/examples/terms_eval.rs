//! 用語の判定と翻訳を、モデルごとにアプリと同じ問い合わせ（`Enricher::terms_and_translations`）で比べる
//! （DESIGN.md 9.3）。文は `experiments/results/entity_llm.json` の 37 文で、正解は GPT-6.1 Terra が
//! 3 回とも同じに答え、確かめた用語の区切り（DESIGN.md 9.1）。
//!
//! usage: OPENAI_API_KEY=… ANTHROPIC_API_KEY=… cargo run -p overlays-core --example terms_eval -- MODEL... [--repeat N]
//!   出力: experiments/results/terms_eval.json（モデルごとに足す。同じモデルは置き換える）
//!   辞書はアプリの保存した分（`{データディレクトリ}/assets`）を使う。
//!   `claude-` で始まるモデルは Anthropic の Messages API に、同じ問い合わせ（`Enricher::question`）を
//!   構造化出力で送る。推論は止める（OpenAI の `reasoning_effort: none` に合わせる）

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use overlays_core::assets;
use overlays_core::enrich::{Enricher, Question, Span};
use overlays_core::openai::Access;
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
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repeat" => repeat = args.next().context("--repeat N")?.parse()?,
            model => models.push(model.to_owned()),
        }
    }
    anyhow::ensure!(
        !models.is_empty(),
        "usage: terms_eval MODEL... [--repeat N]"
    );
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    let http = reqwest::Client::new();

    let cases: Vec<Value> = serde_json::from_slice(&std::fs::read(CASES)?)?;
    let languages: Vec<String> = ["ja"].into_iter().chain(TARGETS).map(Into::into).collect();
    let loaded = assets::load(
        &overlays_core::default_data_dir().join("assets"),
        &languages,
    )
    .await?;
    let glossary = loaded.glossary;

    let mut report: BTreeMap<String, Value> = std::fs::read(OUT)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    for model in &models {
        let meter = Arc::new(Meter::new(
            std::env::temp_dir().join("terms_eval_usage.json"),
        ));
        let enricher = Enricher::new(
            Access::key(key.clone()),
            &Models {
                translate: model.clone(),
                ..Models::default()
            },
            "ja".into(),
            TARGETS.map(Into::into).to_vec(),
            true,
            meter.clone(),
        );
        let claude = model.starts_with("claude-");
        let mut runs = Vec::new();
        let (mut correct, mut leftovers, mut stray, mut failed, mut total_ms) = (0, 0, 0, 0, 0u128);
        let mut claude_usd = 0.0;
        for case in &cases {
            let text = case["text"].as_str().context("text")?;
            let context: Vec<String> = serde_json::from_value(case["context"].clone())?;
            let expected = expected_terms(case);
            for _ in 0..repeat {
                let started = Instant::now();
                let answer = if claude {
                    let question = enricher
                        .question(text, &context, Some(&glossary))
                        .context("nothing to ask")?;
                    ask_claude(&http, &anthropic_key, model, question)
                        .await
                        .and_then(|(answer, usd)| {
                            claude_usd += usd;
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
        let usage = if claude {
            json!({ "translate": { "usd": claude_usd } })
        } else {
            serde_json::to_value(meter.watch().borrow().session.clone())?
        };
        let summary = json!({
            "runs": n,
            "termsCorrect": correct,
            "failed": failed,
            "japaneseLeftInTranslation": leftovers,
            "strayTermsInTranslation": stray,
            "averageMs": total_ms / n as u128,
            "usage": usage,
        });
        println!("{model}: {summary}");
        report.insert(model.clone(), json!({ "summary": summary, "runs": runs }));
    }
    std::fs::write(Path::new(OUT), serde_json::to_vec_pretty(&report)?)?;
    println!("wrote {OUT}");
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

fn has_japanese(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c, '\u{3040}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}'))
}
