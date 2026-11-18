//! 文の数字と句読点を、アプリと同じく整える（`tidy.rs`）。測定（`experiments/tidy_eval.py`）の入力を作る。
//!
//! usage: cargo run -p overlays-core --example tidy -- TEXTS.json
//!   入力: 文の配列。出力: 同じ順に、整えた文の配列（JSON）

use overlays_core::reading::Furigana;
use overlays_core::tidy::tidy;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("TEXTS.json");
    let texts: Vec<String> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let furigana = Furigana::shared().expect("dictionary");
    let out: Vec<String> = texts.iter().map(|text| tidy(furigana, text)).collect();
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
