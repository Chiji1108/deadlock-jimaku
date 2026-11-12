//! ふりがなを、辞書・判定モデルに候補から選ばせる形・LLM に直させる形で比べる（DESIGN.md 6.4）。
//!
//! usage: OPENROUTER_API_KEY=… OPENAI_API_KEY=… cargo run -p overlays-core --example furigana_eval -- CASES.json REPEAT [HOW]
//!   HOW は choose（既定。判定モデルに候補から選ばせる）か、直させるモデルの名前（gpt-6.1-terra、gpt-6-luna）
//!   CASES.json は `experiments/data/furigana_cases.json`（語ごとの正しい読みがある）か、`text` のある文の配列
//!   （正しい読みがないので、GPT-6.1 Terra に直させた読みを基準にして、違うところを出す）

use std::collections::BTreeMap;

use overlays_core::access::Access;
use overlays_core::furigana::{self, Judge, Model, to_hiragana};
use overlays_core::reading::Furigana;
use overlays_core::secrets::Provider;
use overlays_core::types::RubySegment;
use overlays_core::usage::Meter;
use serde_json::Value;

const JUDGE: &str = "perplexity/pplx-decider-v1.2";

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
    let openrouter = Access::key(Provider::OpenRouter, env("OPENROUTER_API_KEY"));
    let openai = Access::key(Provider::OpenAi, env("OPENAI_API_KEY"));
    let meter = Meter::new(std::env::temp_dir().join("overlays-furigana-eval-usage.json"));
    let judge = Judge {
        deadlock: true,
        access: &openrouter,
        model: JUDGE,
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
            if how != "choose" || !alternatives.is_empty() {
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
    Ok(())
}
