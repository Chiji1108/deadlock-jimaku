//! 文を辞書で語に分けて、漢字を含む語の読みを出す。ふりがなの測定（`experiments/chrome_eval.mjs furigana`）の入力にする。
//!
//! usage: cargo run -p overlays-core --example readings -- SENTENCES.json
//!   入力: 文の配列。出力: 文ごとの `{ text, words: [{ text, reading }] }`（読みは漢字を含む語だけ）

use overlays_core::furigana::is_kanji;
use overlays_core::reading::Furigana;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let path = std::env::args().nth(1).expect("SENTENCES.json");
    let sentences: Vec<String> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let furigana = Furigana::shared().expect("dictionary");
    let out: Vec<_> = sentences
        .iter()
        .map(|text| {
            let words: Vec<_> = furigana
                .morphemes(text)
                .into_iter()
                .map(|m| {
                    let word = &text[m.start..m.end];
                    let reading = m.reading.filter(|_| word.chars().any(is_kanji));
                    json!({ "text": word, "reading": reading })
                })
                .collect();
            json!({ "text": text, "words": words })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
