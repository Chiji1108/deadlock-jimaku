//! 文を辞書で語に分けて、読みと、読みが分かれる語の候補を出す。ふりがなの測定（`experiments/chrome_eval.mjs`）の入力にする。
//!
//! usage: cargo run -p overlays-core --example readings -- SENTENCES.json
//!   入力: 文の配列（文字列か、`text` のあるもの）。出力: 文ごとの `{ text, words: [{ text, reading, options, tail }] }`。
//!   `reading` は辞書の読み（漢字を含む語だけ）。`options` は読みの候補（先頭が辞書の読み。アプリが判定モデルに
//!   選ばせるものと同じ）で、`tail` は候補に付けて見せる、続くかな

use overlays_core::furigana::{is_kanji, to_hiragana};
use overlays_core::reading::{Furigana, read};
use serde_json::{Value, json};

/// アプリが候補に付ける、続くかなの数（`furigana.rs` の `TAIL_CHARS`）
const TAIL_CHARS: usize = 4;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("SENTENCES.json");
    let sentences: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let furigana = Furigana::shared().expect("dictionary");
    let out: Vec<_> = sentences
        .iter()
        .map(|sentence| {
            let text = sentence.as_str().or(sentence["text"].as_str()).expect("text");
            let (morphemes, alternatives) = furigana.alternatives(text);
            let words: Vec<_> = morphemes
                .into_iter()
                .map(|m| {
                    let word = &text[m.start..m.end];
                    let asked = alternatives.iter().find(|a| a.start == m.start);
                    let options = asked.map(|a| {
                        let options = a.options.iter().map(|o| read(text, o));
                        options.collect::<Vec<_>>()
                    });
                    let kana = |c: &char| matches!(c, 'ぁ'..='ゖ' | 'ァ'..='ヴ' | 'ー');
                    let tail: String = text[m.end..].chars().take_while(kana).take(TAIL_CHARS).collect();
                    let reading = m.reading.filter(|_| word.chars().any(is_kanji));
                    json!({ "text": word, "reading": reading, "options": options, "tail": to_hiragana(&tail) })
                })
                .collect();
            json!({ "text": text, "words": words })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
