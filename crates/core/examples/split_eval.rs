//! 「翻訳と用語」を 2 つの問い合わせに分けたら、安いモデルや翻訳専門のモデルでも足りるかを測る（DESIGN.md 9.3）。
//! 文と正解は `terms_eval.rs` と同じ（`experiments/results/entity_llm.json` の 37 文、GPT-6.1 Terra の区切り）。
//!
//! usage: cargo run -p overlays-core --example split_eval -- MODE MODEL... [--repeat N]
//!   MODE:
//!     terms-segments  アプリが翻訳先なしのときに送る問い合わせ（`Enricher::question`。原文をすべて区切って返す）
//!     terms-list      用語だけを並べて返す問い合わせ（原文をつなぎ直さなくてよい）
//!     terms-decisions PC の中で候補を出し（辞書の名前と、LLM に一度だけ作らせた別名の一致、「ウルト」と番号）、
//!                     OpenAI の Decisions API に候補ごとに確かめさせる。辞書の一覧は送らない
//!     translate       正解の用語を `⟦英語名⟧` に置き換えた原文を訳すだけの問い合わせ（辞書の一覧は送らない）
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
use overlays_core::enrich::{Enricher, Question, Span};
use overlays_core::llm;
use overlays_core::secrets::Provider;
use overlays_core::settings::Models;
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
    let mut cases_path = CASES.to_owned();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repeat" => repeat = args.next().context("--repeat N")?.parse()?,
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

    for model in &models {
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
                let started = Instant::now();
                let result = match mode.as_str() {
                    "terms-segments" => {
                        terms_segments(&client, model, &enricher, text, &context, &glossary).await
                    }
                    "terms-list" => {
                        terms_list(&client, model, &enricher, text, &context, &glossary).await
                    }
                    "terms-decisions" => {
                        terms_decisions(&client, model, text, &context, &glossary).await
                    }
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
        let usd = serde_json::to_value(client.meter.watch().borrow().session.clone())?["translate"]
            ["usd"]
            .as_f64()
            .unwrap_or(0.0)
            + *client.other_usd.lock().unwrap();
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

const ALIASES: &str = "experiments/results/aliases.json";
const ALIAS_MODEL: &str = "gpt-6.1-terra";

/// 用語ごとの別名（略称や、英語名のカタカナ）。辞書の版ごとに一度だけ LLM に作らせて置いておく
/// （アプリなら辞書の漢字名の読みと同じく、辞書が変わったときに作り直す）
async fn aliases(
    client: &Client,
    glossary: &Glossary,
) -> anyhow::Result<BTreeMap<String, Vec<String>>> {
    static CACHE: tokio::sync::OnceCell<BTreeMap<String, Vec<String>>> =
        tokio::sync::OnceCell::const_new();
    CACHE
        .get_or_try_init(|| async {
            let saved: Value = std::fs::read(ALIASES)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            if saved["clientVersion"] == glossary.client_version {
                return Ok(serde_json::from_value(saved["aliases"].clone())?);
            }
            let lines: Vec<String> = glossary
                .entities
                .iter()
                .map(|(reference, e)| {
                    let name = |lang: &str| e.names.get(lang).map_or("", String::as_str);
                    format!("{reference} | {:?} | {} | {}", e.kind, name("ja"), name("en"))
                })
                .collect();
            let mut aliases = BTreeMap::new();
            for chunk in lines.chunks(60) {
                let question = Question {
                    system: "You know how Japanese streamers talk about the video game Deadlock. For each term                              (id | kind | Japanese name | English name), list the other ways a Japanese streamer may                              say it aloud, written as speech recognition would write them: abbreviations and nicknames                              of the Japanese name (\"拡マガ\" for \"基礎拡張マガジン\"), and the English name or its                              short form in katakana. Give 0 to 5 per term, most likely first. Do not list the Japanese                              name itself, ordinary everyday words, or generic words that fit many terms (\"ウルト\", numbers).\n"
                        .into(),
                    user: chunk.join("\n"),
                    schema: json!({
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["terms"],
                        "properties": { "terms": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["id", "aliases"],
                                "properties": {
                                    "id": { "type": "string" },
                                    "aliases": { "type": "array", "items": { "type": "string" } },
                                },
                            },
                        } },
                    }),
                };
                // 60 語ぶんの答えは長いので、発話の問い合わせより長く待つ
                let answer: Value = llm::chat_json(
                    &client.http,
                    llm::Request {
                        access: &client.openai,
                        model: ALIAS_MODEL,
                        reasoning_effort: Some("none"),
                        system: &question.system,
                        user: question.user,
                        schema_name: "aliases",
                        schema: question.schema,
                        timeout: Duration::from_secs(120),
                        meter: &Meter::new(std::env::temp_dir().join("split_eval_aliases.json")),
                        purpose: Use::Readings,
                    },
                )
                .await?;
                for term in answer["terms"].as_array().context("no terms")? {
                    let (Some(id), Some(list)) = (term["id"].as_str(), term["aliases"].as_array())
                    else {
                        continue;
                    };
                    if glossary.entities.contains_key(id) {
                        let list = list.iter().filter_map(|a| a.as_str().map(str::to_owned));
                        aliases.insert(id.to_owned(), list.collect());
                    }
                }
            }
            let saved = json!({ "clientVersion": glossary.client_version, "aliases": aliases });
            std::fs::write(ALIASES, serde_json::to_vec_pretty(&saved)?)?;
            Ok(aliases)
        })
        .await
        .cloned()
}

/// 用語かもしれない所。`refs` はその言葉が指しうる用語
struct Candidate {
    start: usize,
    spoken: String,
    refs: Vec<String>,
}

/// 文の中の、辞書の名前か別名と同じ所（長いものを先に当てる）
fn matches(text: &str, surfaces: &[(String, Vec<String>)]) -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        match surfaces.iter().find(|(s, _)| rest.starts_with(s.as_str())) {
            Some((surface, refs)) => {
                found.push(Candidate {
                    start: at,
                    spoken: surface.clone(),
                    refs: refs.clone(),
                });
                at += surface.len();
            }
            None => at += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    found
}

/// 聞き違いや表記のゆれを寄せる（ヴァ → バ など）
fn kana(text: &str) -> Vec<char> {
    text.replace("ヴァ", "バ")
        .replace("ヴィ", "ビ")
        .replace("ヴェ", "ベ")
        .replace("ヴォ", "ボ")
        .replace('ヴ', "ブ")
        .chars()
        .collect()
}

/// 最長共通部分列の長さ
fn lcs(a: &[char], b: &[char]) -> usize {
    let mut row = vec![0; b.len() + 1];
    for x in a {
        let mut diagonal = 0;
        for (j, y) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = if x == y { diagonal + 1 } else { up.max(row[j]) };
            diagonal = up;
        }
    }
    row[b.len()]
}

/// 名前とも別名とも一致しないカタカナの言葉に、似た名前の用語を当てる（略称、名前の一部、聞き違い）。
/// 言葉の字の 3 分の 2 以上（2 字以上）が順に名前に入っているものを、似ている順に 5 つまで
fn similar(word: &str, surfaces: &[(String, Vec<String>)]) -> Vec<String> {
    let word = kana(word);
    let mut scored: Vec<(usize, usize, &String)> = Vec::new();
    for (surface, refs) in surfaces {
        let name = kana(surface);
        let common = lcs(&word, &name);
        if common >= 2 && common * 3 >= word.len() * 2 {
            // 言葉をよく説明し、名前の長さが近いものを先に
            let extra = name.len() - common;
            scored.extend(refs.iter().map(|r| (word.len() - common, extra, r)));
        }
    }
    scored.sort();
    let mut refs: Vec<String> = Vec::new();
    for (_, _, r) in scored {
        if refs.len() < 5 && !refs.contains(r) {
            refs.push(r.clone());
        }
    }
    refs
}

/// 文の中の、候補に入っていないカタカナの言葉（2 字以上）
fn katakana_words(text: &str, candidates: &[Candidate]) -> Vec<(usize, String)> {
    let mut words = Vec::new();
    let mut at = 0;
    while at < text.len() {
        if let Some(c) = candidates
            .iter()
            .find(|c| c.start <= at && at < c.start + c.spoken.len())
        {
            at = c.start + c.spoken.len();
            continue;
        }
        let len: usize = text[at..]
            .char_indices()
            .take_while(|(i, ch)| {
                matches!(ch, '\u{30a1}'..='\u{30fc}')
                    && !candidates.iter().any(|c| c.start == at + i)
            })
            .map(|(_, ch)| ch.len_utf8())
            .sum();
        if len == 0 {
            at += text[at..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        let word = &text[at..at + len];
        if word.chars().count() >= 2 {
            words.push((at, word.to_owned()));
        }
        at += len;
    }
    words
}

/// スキルの呼び方（「ウルト」は 4 番、「2」「３番」はその番号）。何番かと、話した言葉の長さ
fn ability_word(rest: &str) -> Option<(u8, usize)> {
    for word in ["ウルト", "ult", "アルティメット"] {
        if rest.starts_with(word) {
            return Some((4, word.len()));
        }
    }
    let first = rest.chars().next()?;
    let slot = match first {
        '1'..='4' => first as u8 - b'0',
        '１'..='４' => (first as u32 - '１' as u32) as u8 + 1,
        _ => return None,
    };
    let after = &rest[first.len_utf8()..];
    Some((
        slot,
        first.len_utf8()
            + if after.starts_with('番') {
                '番'.len_utf8()
            } else {
                0
            },
    ))
}

/// PC の中で候補を出し、Decisions API に候補ごとに「どの用語か、用語ではないか」を選ばせる
async fn terms_decisions(
    client: &Client,
    model: &str,
    text: &str,
    context: &[String],
    glossary: &Glossary,
) -> anyhow::Result<Outcome> {
    // 別名を作る（読む）時間は、発話の応答の速さに数えない
    let preparing = Instant::now();
    let aliases = aliases(client, glossary).await?;
    *client.retried.lock().unwrap() += preparing.elapsed();
    let mut by_surface: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (reference, e) in &glossary.entities {
        let names = e
            .names
            .get("ja")
            .into_iter()
            .chain(aliases.get(reference).into_iter().flatten());
        for name in names.filter(|n| n.chars().count() >= 2) {
            let refs = by_surface.entry(name.clone()).or_default();
            if !refs.contains(reference) {
                refs.push(reference.clone());
            }
        }
    }
    let mut surfaces: Vec<(String, Vec<String>)> = by_surface.into_iter().collect();
    surfaces.sort_by_key(|(s, _)| std::cmp::Reverse(s.len()));

    let mut candidates = matches(text, &surfaces);
    for (start, spoken) in katakana_words(text, &candidates) {
        // 「ウルト」は下で、ヒーローのスキルから選ばせる
        let refs = match ability_word(&spoken) {
            Some(_) => Vec::new(),
            None => similar(&spoken, &surfaces),
        };
        if !refs.is_empty() {
            candidates.push(Candidate {
                start,
                spoken,
                refs,
            });
        }
    }
    candidates.sort_by_key(|c| c.start);
    // 前の発話でも、似た名前のヒーローまで拾う（「ページ」は Paige の聞き違い）
    let in_context: Vec<Candidate> = context
        .iter()
        .flat_map(|c| {
            let mut found = matches(c, &surfaces);
            for (start, spoken) in katakana_words(c, &found) {
                let refs = similar(&spoken, &surfaces);
                found.push(Candidate {
                    start,
                    spoken,
                    refs,
                });
            }
            found
        })
        .collect();
    // 文と前の発話に出たヒーロー（その言葉が指しうるものを全部）。「ウルト」と番号は、そのヒーローのスキルから選ばせる
    let heroes: Vec<&String> = in_context
        .iter()
        .chain(&candidates)
        .flat_map(|c| &c.refs)
        .filter_map(|r| glossary.entities.get_key_value(r))
        .filter(|(_, e)| e.kind == EntityKind::Hero)
        .map(|(r, _)| r)
        .collect();
    let mut at = 0;
    while at < text.len() {
        let covered = candidates
            .iter()
            .find(|c| c.start <= at && at < c.start + c.spoken.len());
        if let Some(c) = covered {
            at = c.start + c.spoken.len();
            continue;
        }
        let rest = &text[at..];
        let Some((slot, len)) = ability_word(rest) else {
            at += rest.chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let refs: Vec<String> = glossary
            .entities
            .iter()
            .filter(|(_, e)| {
                e.ability_slot == Some(slot) && e.hero.as_ref().is_some_and(|h| heroes.contains(&h))
            })
            .map(|(r, _)| r.clone())
            .collect();
        if !refs.is_empty() {
            candidates.push(Candidate {
                start: at,
                spoken: rest[..len].to_owned(),
                refs,
            });
        }
        at += len;
    }
    candidates.sort_by_key(|c| c.start);
    if candidates.is_empty() {
        return Ok(Outcome::Terms(Vec::new()));
    }

    let describe = |reference: &str| {
        let e = &glossary.entities[reference];
        let name = |lang: &str| e.names.get(lang).map_or("", String::as_str);
        let owner = e
            .hero
            .as_deref()
            .and_then(|h| english(glossary, h))
            .zip(e.ability_slot)
            .map_or(String::new(), |(h, slot)| {
                format!(", ability {slot} of {h}")
            });
        let kind = match e.kind {
            EntityKind::Hero => "hero",
            EntityKind::Item => "item",
            EntityKind::Ability => "ability",
        };
        format!("{} ({}): Deadlock {kind}{owner}", name("en"), name("ja"))
    };
    let questions: Vec<Value> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut choices: Vec<Value> = c
                .refs
                .iter()
                .map(|r| json!({ "value": r, "description": describe(r) }))
                .collect();
            choices.push(json!({
                "value": "none",
                "description": "None of these: an ordinary word, part of another word, or it cannot be told which one is meant",
            }));
            let before = &text[..c.start];
            json!({
                "type": "choice",
                "name": format!("c{i}"),
                "instructions": format!(
                    "In the transcript, right after \"{before}\", the streamer says \"{}\". What do they mean by it? \
                     Speech recognition may have misheard it, and streamers use nicknames and abbreviations. \
                     An ability is called by its number or \"ult\" (ability 4) after its hero; use the context to tell whose it is.",
                    c.spoken
                ),
                "choices": choices,
            })
        })
        .collect();
    let body = json!({
        "model": model,
        "input": format!(
            "Live commentary from a Japanese streamer playing the video game Deadlock, written by speech recognition.\n\
             Previous utterances (context only): {}\nTranscript: {text}",
            serde_json::to_string(context)?
        ),
        "questions": questions,
    });
    let res = client
        .http
        .post(client.openai.url("/decisions"))
        .bearer_auth(client.openai.token())
        .timeout(TIMEOUT)
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    let res: Value = res.json().await?;
    anyhow::ensure!(status.is_success(), "{status}: {res}");
    // 入力 100 万トークン $0.10。出力は数えない
    client.spent(res["usage"]["input_tokens"].as_f64().unwrap_or(0.0) * 0.10 / 1e6);
    let answers = res["answers"].as_array().context("no answers")?;
    let mut terms: Vec<(String, String)> = Vec::new();
    let mut last_end = usize::MAX;
    for (i, c) in candidates.iter().enumerate() {
        let name = format!("c{i}");
        let choice = answers
            .iter()
            .find(|a| a["name"] == name.as_str())
            .and_then(|a| a["choice"].as_str());
        let Some(reference) = choice.filter(|r| c.refs.iter().any(|x| x == r)) else {
            continue;
        };
        match terms.last_mut() {
            Some((s, r)) if r == reference && last_end == c.start => s.push_str(&c.spoken),
            _ => terms.push((c.spoken.clone(), reference.to_owned())),
        }
        last_end = c.start + c.spoken.len();
    }
    Ok(Outcome::Terms(terms))
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
