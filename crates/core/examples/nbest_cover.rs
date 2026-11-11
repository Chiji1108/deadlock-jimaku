//! 辞書（Lindera）の上位 N 個の分け方の読みに、正しい読みが入っているかを数える（DESIGN.md 6.4）。
//! 入っていれば、判定モデルに「候補から選ばせる」形で直せる。
//!
//! usage: cargo run -p overlays-core --example nbest_cover -- experiments/data/furigana_cases.json N
//!   標準エラーに、文ごとの候補を 1 行ずつの JSON で出す

use std::borrow::Cow;
use std::collections::BTreeSet;

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use overlays_core::furigana::to_hiragana;
use serde_json::Value;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let cases: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(args.next().expect("CASES.json"))?)?;
    let n: usize = args.next().expect("N").parse()?;
    let segmenter = Segmenter::new(Mode::Normal, load_dictionary("embedded://ipadic")?, None);
    let (mut covered, mut wrong, mut wrong_covered, mut candidates) = (0, 0, 0, 0);
    for case in &cases {
        let text = case["text"].as_str().unwrap();
        // 正しい読み（語ごとの候補の組み合わせ）
        let mut gold = vec![String::new()];
        let mut dictionary = String::new();
        for w in case["words"].as_array().unwrap() {
            let own = w["reading"].as_str().unwrap_or(w["text"].as_str().unwrap());
            dictionary += &to_hiragana(own);
            let alts: Vec<&str> = match w["accept"].as_array() {
                Some(a) => a.iter().filter_map(Value::as_str).collect(),
                None => vec![own],
            };
            gold = gold
                .iter()
                .flat_map(|g| alts.iter().map(move |a| format!("{g}{}", to_hiragana(a))))
                .collect();
        }
        let mut readings = BTreeSet::new();
        for (mut tokens, _) in segmenter.segment_nbest(Cow::Borrowed(text), n, false, None)? {
            let reading: String = tokens
                .iter_mut()
                .map(|t| {
                    let surface = t.surface.to_string();
                    let reading = t
                        .details()
                        .get(7)
                        .filter(|r| **r != "*")
                        .map(|r| (*r).to_owned());
                    to_hiragana(&reading.unwrap_or(surface))
                })
                .collect();
            readings.insert(reading);
        }
        // 判定モデルの測定（`experiments/furigana_choose.py`）に渡す: 文、辞書の読み、候補、正しい読み
        eprintln!(
            "{}",
            serde_json::json!({ "text": text, "dictionary": dictionary, "candidates": readings, "gold": gold })
        );
        let has = gold.iter().any(|g| readings.contains(g));
        let dictionary_right = gold.contains(&dictionary);
        covered += usize::from(has);
        candidates += readings.len();
        if !dictionary_right {
            wrong += 1;
            wrong_covered += usize::from(has);
            if !has {
                println!("no candidate: {text} ({} readings)", readings.len());
            }
        }
    }
    println!(
        "N={n}: {covered}/{} sentences have the right reading among the candidates; of the {wrong} the dictionary gets wrong, {wrong_covered}; {:.1} distinct readings per sentence",
        cases.len(),
        candidates as f64 / cases.len() as f64
    );
    Ok(())
}
