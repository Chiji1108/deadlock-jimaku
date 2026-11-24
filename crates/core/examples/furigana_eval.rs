//! ふりがなを、辞書・判定モデルに候補から選ばせる形・LLM に直させる形で比べる（DESIGN.md 6.4）。
//!
//! usage: OPENROUTER_API_KEY=… OPENAI_API_KEY=… cargo run -p overlays-core --example furigana_eval -- CASES.json REPEAT [HOW]
//!   HOW は choose（既定。判定モデルに候補から選ばせる）、dictionary（辞書の読みのまま。何も聞かない）、
//!   直させるモデルの名前（gpt-6.1-terra、gpt-6-luna）のどれか
//!   `FURIGANA_EVAL_API=http://127.0.0.1:8765/v1` を付けると、choose は PC の中のモデルに聞く
//!   `FURIGANA_EVAL_JUDGE=会社/モデル` を付けると、choose は OpenRouter のその判定モデルに聞く（既定はアプリと同じ）
//!   CASES.json は `experiments/data/furigana_cases.json`（語ごとの正しい読みがある）か、`text` のある文の配列
//!   （正しい読みがないので、GPT-6.1 Terra に直させた読みを基準にして、違うところを出す）
//!   出力: experiments/results/furigana_eval.json（聞き方とモデルごとに足す。同じものは置き換える）。`summary.right` は
//!   正しく読めた回数（`kept` + `fixed`）、`dictionaryRight` は辞書だけで正しかった回数、`at` は測り終えた時刻（UNIX 秒）

use std::collections::BTreeMap;

use overlays_core::access::Access;
use overlays_core::furigana::{self, Door, Judge, Model, to_hiragana};
use overlays_core::reading::Furigana;
use overlays_core::secrets::Provider;
use overlays_core::types::RubySegment;
use overlays_core::usage::Meter;
use serde_json::{Value, json};

const JUDGE: &str = "perplexity/pplx-decider-v1.2";
const OUT: &str = "experiments/results/furigana_eval.json";

fn reading(segments: &[RubySegment]) -> String {
    let read = segments
        .iter()
        .map(|s| s.ruby.as_deref().unwrap_or(&s.text));
    to_hiragana(&read.collect::<String>())
}

/// 語ごとの正しい読みの組み合わせ。正しい読みのない文なら空
fn gold(case: &Value) -> Vec<String> {
    let mut gold = vec![String::new()];
    for w in case["words"].as_array().into_iter().flatten() {
        let own = w["reading"]
            .as_str()
            .or(w["text"].as_str())
            .unwrap_or_default();
        let alts: Vec<&str> = match w["accept"].as_array() {
            Some(a) => a.iter().filter_map(Value::as_str).collect(),
            None => vec![own],
        };
        gold = gold
            .iter()
            .flat_map(|g| alts.iter().map(move |a| format!("{g}{}", to_hiragana(a))))
            .collect();
    }
    if case["words"].is_array() {
        gold
    } else {
        Vec::new()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut args = std::env::args().skip(1);
    let cases: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(args.next().expect("CASES.json"))?)?;
    let repeat: usize = args.next().expect("REPEAT").parse()?;
    let how = args.next().unwrap_or_else(|| "choose".into());
    let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name}"));
    // `FURIGANA_EVAL_API` があれば、判定は PC の中のモデルに聞く（experiments/llama_decider_server.py。
    // `{API}/openrouter/systemone` に、同じ形の問いを送る）
    let openrouter = match std::env::var("FURIGANA_EVAL_API") {
        Ok(api) => Access::at(Provider::OpenRouter, "local", &api),
        Err(_) => Access::key(Provider::OpenRouter, env("OPENROUTER_API_KEY")),
    };
    let openai = Access::key(Provider::OpenAi, env("OPENAI_API_KEY"));
    let meter = Meter::new(std::env::temp_dir().join("overlays-furigana-eval-usage.json"));
    let judge_model = std::env::var("FURIGANA_EVAL_JUDGE").unwrap_or_else(|_| JUDGE.into());
    let door = Door::Api {
        access: openrouter,
        model: judge_model,
    };
    let judge = Judge {
        deadlock: true,
        door: &door,
        meter: &meter,
    };
    let model = |model| Model {
        deadlock: true,
        access: &openai,
        model,
        reasoning_effort: Some("none"),
        meter: &meter,
    };
    let terra = model("gpt-6.1-terra");
    let fixer = model(&how);
    let dictionary = Furigana::shared().expect("dictionary");
    let http = overlays_core::http();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut times = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for case in &cases {
        let text = case["text"].as_str().expect("text");
        // アプリが出す辞書のふりがな（数字などは読みを付けない）
        let morphemes = dictionary.morphemes(text);
        let own = reading(&overlays_core::reading::annotate(
            text,
            &morphemes,
            0,
            text.len(),
        ));
        let mut gold = gold(case);
        if gold.is_empty() {
            // 正しい読みがない文は、Terra に直させた読みを基準にする（聞けなければ、もう一度）
            let mut asked = furigana::ask(http, &terra, dictionary, text, &[]).await;
            for _ in 0..2 {
                if asked.is_err() {
                    asked = furigana::ask(http, &terra, dictionary, text, &[]).await;
                }
            }
            gold.push(reading(&asked?));
        }
        let (_, alternatives) = dictionary.alternatives(text);
        *counts.entry("questions").or_default() += alternatives.len();
        for _ in 0..repeat {
            let started = std::time::Instant::now();
            // 聞けなかったら、アプリと同じく辞書の読みのまま
            let answer = match how.as_str() {
                "choose" => furigana::choose(http, &judge, dictionary, text, &[]).await,
                "dictionary" => Ok(overlays_core::reading::annotate(
                    text,
                    &morphemes,
                    0,
                    text.len(),
                )),
                _ => furigana::ask(http, &fixer, dictionary, text, &[]).await,
            };
            let chosen = match answer {
                Ok(segments) => reading(&segments),
                Err(e) => {
                    *counts.entry("failed").or_default() += 1;
                    eprintln!("failed: {e:#}");
                    own.clone()
                }
            };
            // 選ばせる形は、読みが分かれる語がなければ聞かない
            if how != "dictionary" && (how != "choose" || !alternatives.is_empty()) {
                times.push(started.elapsed().as_millis());
            }
            let outcome = match (gold.contains(&own), gold.contains(&chosen)) {
                (true, true) => "kept",
                (false, true) => "fixed",
                (true, false) => "broken",
                (false, false) => "still wrong",
            };
            *counts.entry(outcome).or_default() += 1;
            if outcome != "kept" && seen.insert((text, chosen.clone())) {
                println!(
                    "{outcome}: {text}\n  dictionary {own}\n  chosen     {chosen}\n  reference  {}",
                    gold[0]
                );
            }
        }
    }
    times.sort_unstable();
    println!("{counts:?}");
    if let Some(median) = times.get(times.len() / 2) {
        println!("asked {} times, median {median} ms", times.len());
    }
    println!("{:?}", meter.watch().borrow().session);
    let session = serde_json::to_value(meter.watch().borrow().session.clone())?;
    let usd: f64 = session
        .as_object()
        .into_iter()
        .flat_map(|uses| uses.values())
        .filter_map(|amount| amount["usd"].as_f64())
        .sum();
    let count = |name: &str| counts.get(name).copied().unwrap_or(0);
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let summary = json!({
        "at": at,
        "runs": cases.len() * repeat,
        "right": count("kept") + count("fixed"),
        "dictionaryRight": count("kept") + count("broken"),
        "fixed": count("fixed"),
        "broken": count("broken"),
        "failed": count("failed"),
        "asked": times.len(),
        "medianMs": times.get(times.len() / 2),
        "p90Ms": times.get(times.len() * 9 / 10),
        "usd": usd,
    });
    let key = match how.as_str() {
        "choose" => format!(
            "choose/{}",
            match &door {
                Door::Api { model, .. } => model,
                Door::Server(_) => unreachable!("made above"),
            }
        ),
        "dictionary" => how.clone(),
        model => format!("fix/{model}"),
    };
    println!("{key}: {summary}");
    // 別々に走らせても消し合わないよう、書く直前に読み直す
    let mut report: BTreeMap<String, Value> = std::fs::read(OUT)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    report.insert(key, json!({ "summary": summary }));
    std::fs::write(OUT, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
