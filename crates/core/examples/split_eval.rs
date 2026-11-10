//! 「翻訳と用語」を 2 つの問い合わせに分けたら、安いモデルや翻訳専門のモデルでも足りるかを測る（DESIGN.md 9.3）。
//! 文と正解は `terms_eval.rs` と同じ（`experiments/results/entity_llm.json` の 37 文、GPT-6.1 Terra の区切り）。
//!
//! usage: cargo run -p overlays-core --example split_eval -- MODE MODEL... [--repeat N]
//!   MODE:
//!     terms-segments  アプリが翻訳先なしのときに送る問い合わせ（`Enricher::question`。原文をすべて区切って返す）
//!     terms-list      用語だけを並べて返す問い合わせ（原文をつなぎ直さなくてよい）
//!     terms-decisions アプリと同じ処理（`decide::Decider`）。PC の中で候補を出し、OpenAI の Decisions API に確かめさせる
//!     terms-jev       アプリと同じ候補を、TypeSafe AI の Jev に確かめさせる（MODEL は `jev-latest` など）。
//!                     `会社/モデル` の形なら、OpenRouter の判定モデル（`perplexity/pplx-decider-v1.2` など）に送る
//!     pipeline        アプリと同じ処理で、用語を決めてから訳す（`Decider` → `mt::Translator`）。MODEL は名前だけ
//!     translate       正解の用語を `⟦英語名⟧` に置き換えた原文を訳すだけの問い合わせ（辞書の一覧は送らない）
//!   `SPLIT_EVAL_ASSETS` で辞書の置き場所を替えられる（別名のキャッシュをアプリの置き場所に書かないとき）。
//!   出力: experiments/results/split_eval.json（"MODE/MODEL" ごとに足す。同じものは置き換える）
//!   `claude-` は Anthropic、`qwen` は QwenCloud（シンガポール）、ほかは OpenAI に送る。どれも推論は止める。
//!   `qwen-mt-` は翻訳専門のモデルで、translate だけ。翻訳先ごとに 1 回ずつ同時に送る。
//!   `deepl`（既定のモデル）と `deepl-latency`（速さを優先）は DeepL の翻訳 API（`DEEPL_API_KEY`）で、これも translate だけ

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use overlays_core::access::Access;
use overlays_core::assets;
use overlays_core::assets::Loaded;
use overlays_core::decide::Decider;
use overlays_core::enrich::{Enricher, Question, Span};
use overlays_core::llm;
use overlays_core::mt::Translator;
use overlays_core::secrets::Provider;
use overlays_core::settings::Models;
use overlays_core::settings::Settings;
use overlays_core::store::Store;
use overlays_core::types::Token;
use overlays_core::types::{EntityKind, Glossary};
use overlays_core::usage::{Meter, Use};
use serde_json::{Value, json};

const CASES: &str = "experiments/results/entity_llm.json";
const OUT: &str = "experiments/results/split_eval.json";
const TARGETS: [(&str, &str); 2] = [("en", "English"), ("ru", "Russian")];
const TIMEOUT: Duration = Duration::from_secs(8);
const QWEN: &str =
    "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1/chat/completions";

/// 送り先と、料金の数え方
struct Client {
    http: reqwest::Client,
    openai: Access,
    anthropic_key: String,
    qwen_key: String,
    deepl_key: String,
    /// OpenAI の分（アプリと同じ単価で数える）
    meter: Meter,
    /// ほかの会社の分（ドル）
    other_usd: std::sync::Mutex<f64>,
    /// 断られてやり直した分の時間（応答の速さに数えない）
    retried: std::sync::Mutex<Duration>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut args = std::env::args().skip(1);
    let mode = args
        .next()
        .context("usage: split_eval MODE MODEL... [--repeat N]")?;
    let mut models = Vec::new();
    let mut repeat = 3;
    let mut pace = Duration::ZERO;
    let mut cases_path = CASES.to_owned();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repeat" => repeat = args.next().context("--repeat N")?.parse()?,
            // 発話の間を空ける（ミリ秒）。続けて送ると、回数の上限に当たる会社がある
            "--pace" => pace = Duration::from_millis(args.next().context("--pace MS")?.parse()?),
            // 別の文と正解で測る（`experiments/terms_cases.py` の出力）。結果は文の名前を付けた別のファイルに書く
            "--cases" => cases_path = args.next().context("--cases PATH")?,
            model => models.push(model.to_owned()),
        }
    }
    anyhow::ensure!(!models.is_empty(), "no models");
    let env = |name: &str| std::env::var(name).unwrap_or_default();

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
    let languages: Vec<String> = ["ja", "en", "ru"].map(Into::into).to_vec();
    let glossary = assets::load(
        &overlays_core::default_data_dir().join("assets"),
        &languages,
    )
    .await?
    .glossary;

    let root = overlays_core::default_data_dir().join("assets");
    let app = App::new(
        std::env::var("SPLIT_EVAL_ASSETS").map_or(root, Into::into),
        Access::key(Provider::OpenAi, env("OPENAI_API_KEY")),
        Access::key(Provider::Qwen, env("QWEN_SOUTH_API_KEY")),
    )
    .await?;
    for model in &models {
        let app_usd_before = translate_usd(&app.meter)?;
        let client = Client {
            http: reqwest::Client::new(),
            openai: Access::key(Provider::OpenAi, env("OPENAI_API_KEY")),
            anthropic_key: env("ANTHROPIC_API_KEY"),
            qwen_key: env("QWEN_SOUTH_API_KEY"),
            deepl_key: env("DEEPL_API_KEY"),
            meter: Meter::new(std::env::temp_dir().join("split_eval_usage.json")),
            other_usd: Default::default(),
            retried: Default::default(),
        };
        // 翻訳先なしの Enricher は、用語だけを聞く
        let enricher = Enricher::new(
            client.openai.clone(),
            &Models::default(),
            "ja".into(),
            Vec::new(),
            true,
            Arc::new(Meter::new(
                std::env::temp_dir().join("split_eval_unused.json"),
            )),
        );
        let mut runs = Vec::new();
        let (mut ok_count, mut failed) = (0, 0);
        let mut times = Vec::new();
        for case in &cases {
            let text = case["text"].as_str().context("text")?;
            let context: Vec<String> = serde_json::from_value(case["context"].clone())?;
            for _ in 0..repeat {
                tokio::time::sleep(pace).await;
                let started = Instant::now();
                let result = match mode.as_str() {
                    "terms-segments" => {
                        terms_segments(&client, model, &enricher, text, &context, &glossary).await
                    }
                    "terms-list" => {
                        terms_list(&client, model, &enricher, text, &context, &glossary).await
                    }
                    "terms-decisions" => terms_decisions(&app, text, &context).await,
                    "terms-jev" => terms_jev(&client, &app, model, text, &context).await,
                    "pipeline" => pipeline(&app, text, &context).await,
                    "translate" => translate(&client, model, case, &context, &glossary).await,
                    other => anyhow::bail!("unknown mode: {other}"),
                };
                let retried = std::mem::take(&mut *client.retried.lock().unwrap());
                let ms = started.elapsed().saturating_sub(retried).as_millis() as u64;
                times.push(ms);
                match result {
                    Ok(Outcome::Terms(terms)) => {
                        let expected = expected_terms(case);
                        let ok = terms == expected;
                        ok_count += usize::from(ok);
                        runs.push(json!({ "text": text, "ms": ms, "ok": ok, "terms": terms, "expected": expected }));
                    }
                    Ok(Outcome::Pipeline(terms, translations)) => {
                        let expected = expected_terms(case);
                        let ok = terms == expected;
                        ok_count += usize::from(ok);
                        // 訳文の崩れは、見つけた用語（正解ではなく）に対して数える
                        let names: Vec<String> = terms
                            .iter()
                            .filter_map(|(_, r)| english(&glossary, r).map(str::to_owned))
                            .collect();
                        let problems: BTreeMap<&str, Vec<String>> = translations
                            .iter()
                            .map(|(lang, t)| (*lang, problems(t, &names)))
                            .collect();
                        runs.push(json!({
                            "text": text, "ms": ms, "ok": ok, "terms": terms, "expected": expected,
                            "translations": translations, "problems": problems,
                        }));
                    }
                    Ok(Outcome::Translations(translations)) => {
                        let names = expected_names(case, &glossary);
                        let problems: BTreeMap<&str, Vec<String>> = translations
                            .iter()
                            .map(|(lang, t)| (*lang, problems(t, &names)))
                            .collect();
                        let ok = problems.values().all(Vec::is_empty);
                        ok_count += usize::from(ok);
                        runs.push(json!({
                            "text": text, "ms": ms, "ok": ok,
                            "source": tagged(case, &glossary), "translations": translations, "problems": problems,
                        }));
                    }
                    Err(e) => {
                        failed += 1;
                        runs.push(json!({ "text": text, "ms": ms, "error": format!("{e:#}") }));
                    }
                }
            }
        }
        times.sort_unstable();
        // アプリの部品（Decider と Translator）は、アプリの数え方で数える。このモデルの分だけを取る
        let usd = translate_usd(&client.meter)?
            + *client.other_usd.lock().unwrap()
            + (translate_usd(&app.meter)? - app_usd_before);
        let summary = json!({
            "runs": times.len(),
            "ok": ok_count,
            "failed": failed,
            "medianMs": times[times.len() / 2],
            "p90Ms": times[times.len() * 9 / 10],
            "usd": usd,
        });
        println!("{mode}/{model}: {summary}");
        // モデルごとに別々に走らせても消し合わないよう、書く直前に読み直す
        let mut report: BTreeMap<String, Value> = std::fs::read(&out)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        report.insert(
            format!("{mode}/{model}"),
            json!({ "summary": summary, "runs": runs }),
        );
        std::fs::write(Path::new(&out), serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}

enum Outcome {
    /// 見つけた用語（話した言葉と参照）
    Terms(Vec<(String, String)>),
    /// 訳文。用語は `⟦英語名⟧` のまま
    Translations(BTreeMap<&'static str, String>),
    /// 見つけた用語と、その用語で訳した文
    Pipeline(Vec<(String, String)>, BTreeMap<&'static str, String>),
}

/// アプリが翻訳先なしのときに送る問い合わせ。原文をすべて区切って返させ、つなげて原文にならなければ用語を付けない
async fn terms_segments(
    client: &Client,
    model: &str,
    enricher: &Enricher,
    text: &str,
    context: &[String],
    glossary: &Glossary,
) -> anyhow::Result<Outcome> {
    let question = enricher
        .question(text, context, Some(glossary))
        .context("nothing to ask")?;
    let answer = client.ask(model, question).await?;
    let (spans, _) = enricher.read(text, Some(glossary), answer)?;
    Ok(Outcome::Terms(
        spans
            .into_iter()
            .filter_map(|s| match s {
                Span::Entity { reference, spoken } => Some((spoken, reference)),
                Span::Text(_) => None,
            })
            .collect(),
    ))
}

/// 用語だけを、出てくる順に並べて返させる。原文の中に順に見つかったものだけを使い、隣り合う同じ用語は 1 つにする
async fn terms_list(
    client: &Client,
    model: &str,
    enricher: &Enricher,
    text: &str,
    context: &[String],
    glossary: &Glossary,
) -> anyhow::Result<Outcome> {
    // 用語の一覧は、アプリの問い合わせと同じ並べ方のものを使う
    let app = enricher
        .question(text, context, Some(glossary))
        .context("nothing to ask")?;
    let list = &app.system[app.system.find("Glossary (").context("no glossary")?..];
    let system = format!(
        "You find game terms in live commentary from a streamer playing the video game Deadlock. \
         The streamer speaks Japanese. Each request has \"transcript\" (one utterance from speech recognition, \
         which may mishear words; the streamer may also mispronounce names) and \"context\" \
         (the previous utterances, for reference only; do not list their terms).\n\
         \n\
         In \"terms\", list every part of the transcript that names a game term (a hero, item or ability in the \
         glossary below), in the order they appear: \"text\" is the words copied exactly from the transcript, \
         \"ref\" is the term's id. Recognize nicknames, abbreviations and misheard or mispronounced names, \
         and abilities called by their number, by \"ult\" (ability 4) or by part of their name, after their hero \
         (\"グレイブスの3番\" is Graves' ability 3, \"ドアマンのカート\" is Doorman's Luggage Cart). \
         Use the context to tell whose ability it is. List only what the streamer really means as a game term; \
         ordinary words are not terms (\"1番強い\", a web page, a shopping cart). A term's text holds only the words \
         that name it (for \"グレイブスの3番\": \"グレイブス\" is the hero and \"3番\" is the ability; \"の\" is not listed). \
         If the transcript names no game term, answer with an empty list.\n\
         \n\
         {list}"
    );
    let question = Question {
        system: system.into(),
        user: app.user,
        schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["terms"],
            "properties": { "terms": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["text", "ref"],
                    "properties": { "text": { "type": "string" }, "ref": { "type": "string" } },
                },
            } },
        }),
    };
    let answer = client.ask(model, question).await?;
    let mut terms: Vec<(String, String)> = Vec::new();
    let (mut at, mut last_end) = (0, usize::MAX);
    for term in answer["terms"].as_array().context("no terms")? {
        let (Some(spoken), Some(reference)) = (term["text"].as_str(), term["ref"].as_str()) else {
            continue;
        };
        if spoken.is_empty() || !glossary.entities.contains_key(reference) {
            continue;
        }
        let Some(found) = text[at..].find(spoken) else {
            continue;
        };
        let start = at + found;
        at = start + spoken.len();
        match terms.last_mut() {
            Some((s, r)) if r == reference && last_end == start => s.push_str(spoken),
            _ => terms.push((spoken.to_owned(), reference.to_owned())),
        }
        last_end = at;
    }
    Ok(Outcome::Terms(terms))
}

/// アプリと同じ処理（`decide::Decider`）で用語を決める
async fn terms_decisions(app: &App, text: &str, context: &[String]) -> anyhow::Result<Outcome> {
    let spans = app
        .decider
        .terms(overlays_core::http(), text, context, &app.loaded)
        .await?;
    Ok(Outcome::Terms(entities(spans)))
}

/// Jev の単価（100 万トークンあたりのドル。入力、出力）。TypeSafe AI のサイトの「10 億トークン $42、出力は無料」
/// （料金の解説の記事から。料金のページはない）
const JEV_PRICE: (f64, f64) = (0.042, 0.0);

/// Drex Max の単価（OpenRouter に載っている値と同じ。入力 100 万トークン $0.06、出力は無料）
const DREX_MAX_PRICE: (f64, f64) = (0.06, 0.0);

/// アプリと同じ候補と問いを、TypeSafe AI の Jev に確かめさせる（`TYPESAFE_AI_API_KEY`）。
/// 問いの形だけ Jev に合わせる（`state` と、名前ごとの `questions`、選択肢は `criteria`）
async fn terms_jev(
    client: &Client,
    app: &App,
    model: &str,
    text: &str,
    context: &[String],
) -> anyhow::Result<Outcome> {
    let Some(asked) = app.decider.ask(text, context, &app.loaded) else {
        return Ok(Outcome::Terms(Vec::new()));
    };
    let mut questions = serde_json::Map::new();
    for q in asked.request["questions"].as_array().into_iter().flatten() {
        let criteria: serde_json::Map<String, Value> = q["choices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| Some((c["value"].as_str()?.to_owned(), c["description"].clone())))
            .collect();
        questions.insert(
            q["name"].as_str().context("name")?.to_owned(),
            json!({ "type": "choice", "instructions": q["instructions"], "criteria": criteria }),
        );
    }
    let body = json!({ "model": model, "state": asked.request["input"], "questions": questions });
    // `会社/モデル` の形なら OpenRouter（判定モデルをまとめて同じ形で呼べる。`OPENROUTER_API_KEY`）
    // `drex-` は Nace に直接（`NACE_API_KEY`）。どこも同じ形の口
    let (url, key, price) = match model {
        m if m.contains('/') => (
            "https://openrouter.ai/api/v1/systemone",
            "OPENROUTER_API_KEY",
            JEV_PRICE,
        ),
        // `local-` は、この PC で待たせた判定モデル（experiments/local_decider_server.py）。料金はかからない
        m if m.starts_with("local-") => ("http://127.0.0.1:8765/v1/systemone", "", (0.0, 0.0)),
        m if m.starts_with("drex-") => (
            "https://console.nace.ai/v1/systemone",
            "NACE_API_KEY",
            DREX_MAX_PRICE,
        ),
        _ => (
            "https://api.typesafe.ai/v1/systemone",
            "TYPESAFE_AI_API_KEY",
            JEV_PRICE,
        ),
    };
    let res = client
        .http
        .post(url)
        .bearer_auth(std::env::var(key).unwrap_or_default())
        .timeout(TIMEOUT)
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    let res: Value = res.json().await?;
    anyhow::ensure!(status.is_success(), "{status}: {res}");
    let n = |k: &str| res["usage"][k].as_f64().unwrap_or(0.0);
    let (input, output) = price;
    // OpenRouter は料金を答えに入れる
    let cost = res["usage"]["cost"].as_f64();
    client.spent(cost.unwrap_or((n("input_tokens") * input + n("output_tokens") * output) / 1e6));
    let spans = asked.spans(|name| res["answers"][name]["choice"].as_str().map(str::to_owned));
    Ok(Outcome::Terms(entities(spans)))
}

/// アプリと同じ処理で、用語を決めてから訳す（`decide::Decider` → `mt::Translator`）
async fn pipeline(app: &App, text: &str, context: &[String]) -> anyhow::Result<Outcome> {
    let http = overlays_core::http();
    let spans = app.decider.terms(http, text, context, &app.loaded).await?;
    let translations = app
        .translator
        .translate(http, &spans, Some(&app.loaded.glossary))
        .await?;
    let written = |lang: &'static str| {
        let tokens = translations
            .get(lang)
            .with_context(|| format!("no {lang}"))?;
        let text: String = tokens
            .iter()
            .map(|t| match t {
                Token::Text { text, .. } => text.clone(),
                Token::Entity { reference, .. } => {
                    format!(
                        "⟦{}⟧",
                        english(&app.loaded.glossary, reference).unwrap_or("?")
                    )
                }
            })
            .collect();
        anyhow::Ok((lang, text))
    };
    Ok(Outcome::Pipeline(
        entities(spans),
        BTreeMap::from([written("en")?, written("ru")?]),
    ))
}

/// 用語と翻訳に使った額（ドル）
fn translate_usd(meter: &Meter) -> anyhow::Result<f64> {
    let session = serde_json::to_value(meter.watch().borrow().session.clone())?;
    Ok(session["translate"]["usd"].as_f64().unwrap_or(0.0))
}

fn entities(spans: Vec<Span>) -> Vec<(String, String)> {
    spans
        .into_iter()
        .filter_map(|s| match s {
            Span::Entity { reference, spoken } => Some((spoken, reference)),
            Span::Text(_) => None,
        })
        .collect()
}

/// アプリの部品。辞書と別名はアプリと同じ処理（`assets::load_into`）で用意する
struct App {
    /// アプリの部品が使った分（辞書の読みと別名は `Use::Readings` で、ここには入らない）
    meter: Arc<Meter>,
    loaded: Loaded,
    decider: Decider,
    translator: Translator,
}

impl App {
    /// `root` はアプリの辞書の置き場所。別名がまだなければ、ここで LLM に作らせて置く
    async fn new(root: std::path::PathBuf, openai: Access, qwen: Access) -> anyhow::Result<Self> {
        let settings = Settings {
            translate_to: TARGETS.map(|(code, _)| code.to_owned()).to_vec(),
            ..Default::default()
        };
        let dir = std::env::temp_dir().join("split_eval_store");
        let store = Store::new(Ok(settings.clone()), dir);
        assets::load_into(store.clone(), root, Some(openai.clone()), true).await;
        let loaded = store.glossary().context("no glossary")?;
        Ok(Self {
            meter: store.meter(),
            loaded,
            decider: Decider::new(openai, settings.mic_language.clone(), store.meter()),
            translator: Translator::new(
                qwen,
                settings.mic_language,
                settings.translate_to,
                store.meter(),
            ),
        })
    }
}

/// 正解の用語を `⟦英語名⟧` に置き換えた原文を訳させる。辞書の一覧は送らず、その文の用語が何かだけを添える
async fn translate(
    client: &Client,
    model: &str,
    case: &Value,
    context: &[String],
    glossary: &Glossary,
) -> anyhow::Result<Outcome> {
    let source = tagged(case, glossary);
    if model.starts_with("deepl") {
        let asked = [("en", "EN-US"), ("ru", "RU")]
            .map(|(_, target)| client.ask_deepl(model, &source, context, target));
        let [en, ru] = asked;
        let (en, ru) = tokio::try_join!(en, ru)?;
        return Ok(Outcome::Translations(BTreeMap::from([
            ("en", en),
            ("ru", ru),
        ])));
    }
    if model.starts_with("qwen-mt-") {
        let asked = TARGETS.map(|(_, name)| client.ask_qwen_mt(model, &source, name));
        let [en, ru] = asked;
        let (en, ru) = tokio::try_join!(en, ru)?;
        return Ok(Outcome::Translations(BTreeMap::from([
            ("en", en),
            ("ru", ru),
        ])));
    }
    let terms: BTreeMap<String, String> = expected_terms(case)
        .iter()
        .filter_map(|(_, reference)| {
            let e = glossary.entities.get(reference)?;
            let kind = match e.kind {
                EntityKind::Hero => "hero",
                EntityKind::Item => "item",
                EntityKind::Ability => "ability",
            };
            Some((e.names.get("en")?.clone(), kind.to_owned()))
        })
        .collect();
    let question = Question {
        system: "You translate live commentary from a streamer playing the video game Deadlock, who speaks Japanese. \
                 Each request has \"transcript\" (one utterance; its game terms are already replaced by their English \
                 names in brackets, like ⟦Haze⟧), \"terms\" (what each bracketed term is) and \"context\" \
                 (the previous utterances, for reference only; do not translate them).\n\
                 \n\
                 In \"translations\", translate only the transcript into each requested language, naturally and casually, \
                 like gamers talk. Copy each bracketed term exactly as it is, brackets included, where it belongs; \
                 never translate, transliterate or drop it. When a term starts with \"The\", do not put an article right before it. \
                 Do not attach inflectional endings to a term; rephrase instead (Russian: \"ульта ⟦Haze⟧\", not \"⟦Haze⟧а\"). \
                 The only exception is the English possessive 's, which reads naturally: \"⟦Haze⟧'s ⟦Sleep Dagger⟧\". \
                 Keep common gamer words natural (ult, farm, push, gank).\n"
            .into(),
        user: json!({ "context": context, "transcript": source, "terms": terms }).to_string(),
        schema: json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["translations"],
            "properties": { "translations": {
                "type": "object",
                "additionalProperties": false,
                "required": ["en", "ru"],
                "properties": { "en": { "type": "string" }, "ru": { "type": "string" } },
            } },
        }),
    };
    let answer = client.ask(model, question).await?;
    let read = |lang: &'static str| {
        answer["translations"][lang]
            .as_str()
            .map(|t| (lang, t.to_owned()))
            .with_context(|| format!("no {lang}"))
    };
    Ok(Outcome::Translations(BTreeMap::from([
        read("en")?,
        read("ru")?,
    ])))
}

impl Client {
    /// 構造化出力で聞き、答えの JSON を返す
    async fn ask(&self, model: &str, q: Question) -> anyhow::Result<Value> {
        if model.starts_with("claude-") {
            self.ask_claude(model, q).await
        } else if model.starts_with("qwen") {
            self.ask_qwen(model, q).await
        } else {
            llm::chat_json(
                &self.http,
                llm::Request {
                    access: &self.openai,
                    model,
                    reasoning_effort: Some("none"),
                    system: &q.system,
                    user: q.user,
                    schema_name: "analysis",
                    schema: q.schema,
                    timeout: TIMEOUT,
                    meter: &self.meter,
                    purpose: Use::Translate,
                },
            )
            .await
        }
    }

    fn spent(&self, usd: f64) {
        *self.other_usd.lock().unwrap() += usd;
    }

    /// 単価は Claude Haiku 5.5（キャッシュの読み 0.1 倍、5 分の書き込み 1.25 倍）
    async fn ask_claude(&self, model: &str, q: Question) -> anyhow::Result<Value> {
        anyhow::ensure!(model == "claude-haiku-5-5", "unpriced model: {model}");
        let body = json!({
            "model": model,
            "max_tokens": 4096,
            "system": [{ "type": "text", "text": &*q.system, "cache_control": { "type": "ephemeral" } }],
            "messages": [{ "role": "user", "content": q.user }],
            "thinking": { "type": "disabled" },
            "output_config": { "effort": "low", "format": { "type": "json_schema", "schema": q.schema } },
        });
        let res = self
            .http
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.anthropic_key)
            .header("anthropic-version", "2023-06-01")
            .timeout(TIMEOUT)
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
        let n = |k: &str| res["usage"][k].as_f64().unwrap_or(0.0);
        self.spent(
            (n("input_tokens") * 0.10
                + n("cache_read_input_tokens") * 0.01
                + n("cache_creation_input_tokens") * 0.125
                + n("output_tokens") * 0.50)
                / 1e6,
        );
        Ok(serde_json::from_str(text)?)
    }

    /// 単価はシンガポールの 100 万トークンまでの段（キャッシュの読みは 0.1 倍、書き込みは 1.25 倍）
    async fn ask_qwen(&self, model: &str, q: Question) -> anyhow::Result<Value> {
        let (input, output) = qwen_price(model)?;
        let body = json!({
            "model": model,
            "messages": [
                { "role": "system", "content": [
                    { "type": "text", "text": &*q.system, "cache_control": { "type": "ephemeral" } },
                ] },
                { "role": "user", "content": q.user },
            ],
            "enable_thinking": false,
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": "analysis", "strict": true, "schema": q.schema },
            },
        });
        let res = self.qwen(body).await?;
        let text = res["choices"][0]["message"]["content"]
            .as_str()
            .context("no text")?;
        let n = |v: &Value| v.as_f64().unwrap_or(0.0);
        let u = &res["usage"];
        let details = &u["prompt_tokens_details"];
        let (read, written) = (
            n(&details["cached_tokens"]),
            n(&details["cache_creation_input_tokens"]),
        );
        let fresh = n(&u["prompt_tokens"]) - read - written;
        self.spent(
            (fresh * input
                + read * input * 0.1
                + written * input * 1.25
                + n(&u["completion_tokens"]) * output)
                / 1e6,
        );
        Ok(serde_json::from_str(text)?)
    }

    /// 翻訳専門のモデル。user の 1 通だけを受け、言語と分野は `translation_options` で渡す
    async fn ask_qwen_mt(&self, model: &str, source: &str, target: &str) -> anyhow::Result<String> {
        let (input, output) = qwen_price(model)?;
        let body = json!({
            "model": model,
            "messages": [{ "role": "user", "content": source }],
            "translation_options": {
                "source_lang": "Japanese",
                "target_lang": target,
                "domains": "Casual live commentary by a streamer playing the video game Deadlock, like gamers talk. \
                            Words in ⟦ ⟧ are game terms (names of heroes, items and abilities); \
                            keep them exactly as they are, with the brackets.",
            },
        });
        let res = self.qwen(body).await?;
        let n = |k: &str| res["usage"][k].as_f64().unwrap_or(0.0);
        self.spent((n("prompt_tokens") * input + n("completion_tokens") * output) / 1e6);
        Ok(res["choices"][0]["message"]["content"]
            .as_str()
            .context("no text")?
            .to_owned())
    }

    /// DeepL の翻訳 API。用語は `<t>` で囲み、訳さない所として渡す（`ignore_tags`）。前の発話は `context` に渡す
    /// （訳さず、料金にも数えられない）。料金は API Pro の 100 万文字 $25 で数える（月の基本料金は入れない）
    async fn ask_deepl(
        &self,
        model: &str,
        source: &str,
        context: &[String],
        target: &str,
    ) -> anyhow::Result<String> {
        let escaped = source
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('⟦', "<t>")
            .replace('⟧', "</t>");
        let mut body = json!({
            "text": [escaped],
            "source_lang": "JA",
            "target_lang": target,
            "tag_handling": "xml",
            "ignore_tags": ["t"],
            "context": format!(
                "Casual live commentary by a streamer playing the video game Deadlock. \
                 Words in <t> tags are names of heroes, items and abilities. Previous utterances: {}",
                context.join(" ")
            ),
        });
        if model == "deepl-latency" {
            body["model_type"] = json!("latency_optimized");
        }
        // 無料プランの鍵（`:fx` で終わる）は口が違う
        let host = if self.deepl_key.ends_with(":fx") {
            "api-free.deepl.com"
        } else {
            "api.deepl.com"
        };
        // 回数の上限（429。本文は空）は、待ってやり直す
        let mut attempt = 0;
        let res = loop {
            let started = Instant::now();
            let res = self
                .http
                .post(format!("https://{host}/v2/translate"))
                .header(
                    "Authorization",
                    format!("DeepL-Auth-Key {}", self.deepl_key),
                )
                .timeout(TIMEOUT)
                .json(&body)
                .send()
                .await?;
            attempt += 1;
            if res.status() != reqwest::StatusCode::TOO_MANY_REQUESTS || attempt == 10 {
                break res;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
            *self.retried.lock().unwrap() += started.elapsed();
        };
        let status = res.status();
        let text = res.text().await?;
        anyhow::ensure!(status.is_success(), "{status}: {text}");
        let res: Value = serde_json::from_str(&text)?;
        self.spent(source.chars().count() as f64 * 25.0 / 1e6);
        Ok(res["translations"][0]["text"]
            .as_str()
            .context("no text")?
            .replace("<t>", "⟦")
            .replace("</t>", "⟧")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"))
    }

    /// 回数の上限（429）と内容の検査（同じ文でも通ったり断られたりする）は、待ってやり直す
    async fn qwen(&self, body: Value) -> anyhow::Result<Value> {
        let mut attempt = 0;
        let (status, res) = loop {
            let started = Instant::now();
            let res = self
                .http
                .post(QWEN)
                .bearer_auth(&self.qwen_key)
                .timeout(TIMEOUT)
                .json(&body)
                .send()
                .await?;
            let status = res.status();
            let res: Value = res.json().await?;
            let again = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || res["error"]["code"] == "data_inspection_failed";
            attempt += 1;
            if !again || attempt == 8 {
                break (status, res);
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            *self.retried.lock().unwrap() += started.elapsed();
        };
        anyhow::ensure!(status.is_success(), "{status}: {res}");
        let reason = &res["choices"][0]["finish_reason"];
        anyhow::ensure!(reason == "stop", "stopped: {reason}");
        Ok(res)
    }
}

/// 100 万トークンあたりのドル（入力、出力）。シンガポールの単価。
/// 翻訳専門のモデルの単価は料金の比較サイトの値で、QwenCloud の料金ページでは確かめていない
fn qwen_price(model: &str) -> anyhow::Result<(f64, f64)> {
    Ok(match model {
        "qwen3.8-flash" => (0.15, 0.47),
        "qwen-mt-lite" => (0.12, 0.36),
        "qwen-mt-flash" => (0.16, 0.49),
        "qwen-mt-plus" => (2.46, 7.37),
        _ => anyhow::bail!("unpriced model: {model}"),
    })
}

/// 正解の区切り（話した言葉と、用語なら参照）。Terra の 3 回の答えは同じなので、最初の答えを使う
fn segments(case: &Value) -> impl Iterator<Item = (&str, Option<&str>)> {
    case["llm"][0]["answer"]["segments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| Some((s["text"].as_str()?, s["ref"].as_str())))
}

fn expected_terms(case: &Value) -> Vec<(String, String)> {
    segments(case)
        .filter_map(|(text, reference)| Some((text.to_owned(), reference?.to_owned())))
        .collect()
}

fn english<'a>(glossary: &'a Glossary, reference: &str) -> Option<&'a str> {
    Some(glossary.entities.get(reference)?.names.get("en")?.as_str())
}

/// 用語を `⟦英語名⟧` に置き換えた原文
fn tagged(case: &Value, glossary: &Glossary) -> String {
    segments(case)
        .map(
            |(text, reference)| match reference.and_then(|r| english(glossary, r)) {
                Some(name) => format!("⟦{name}⟧"),
                None => text.to_owned(),
            },
        )
        .collect()
}

fn expected_names(case: &Value, glossary: &Glossary) -> Vec<String> {
    expected_terms(case)
        .iter()
        .filter_map(|(_, r)| english(glossary, r).map(str::to_owned))
        .collect()
}

/// 訳文の崩れ。用語が抜けた、知らない用語を書いた、用語に語尾を付けた、日本語が残った、括弧が閉じていない
fn problems(translation: &str, names: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    let mut rest = translation;
    while let Some(start) = rest.find('⟦') {
        let Some(len) = rest[start..].find('⟧') else {
            problems.push("unclosed".to_owned());
            break;
        };
        let name = &rest[start + '⟦'.len_utf8()..start + len];
        rest = &rest[start + len + '⟧'.len_utf8()..];
        if !names.iter().any(|n| n == name) {
            problems.push(format!("unknown: {name}"));
        }
        if rest.chars().next().is_some_and(char::is_alphabetic) {
            problems.push(format!("inflected: {name}"));
        }
        found.push(name);
    }
    for name in names {
        if !found.contains(&name.as_str()) {
            problems.push(format!("missing: {name}"));
        }
    }
    if translation
        .chars()
        .any(|c| matches!(c, '\u{3040}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}'))
    {
        problems.push("japanese".to_owned());
    }
    problems
}
