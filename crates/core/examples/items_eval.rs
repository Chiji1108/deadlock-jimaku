//! 試合の画面の左下のアイテム欄から、持っているアイテムを読めるかを測る（DESIGN.md 15 章「画面からの読み取り」）。
//! 枠は、deadlock-api のショップの画像（`shop_image`）を縮めて、決まった位置に置いたもの。
//! 前に読んだときになかったアイテムが、買ったアイテムになる。
//!
//! usage: cargo run -p overlays-core --release --example items_eval
//!   画面: experiments/data/item_slots/cases.json（画面の左下を切り出した PNG と、上の段の左から順のアイテムの名前。
//!   入っているが名前の分からない枠は "?"）
//!   出力: experiments/results/items_eval.json
//!   ショップの画像は deadlock-api から取り、`target/screen_samples/` に置いておく

mod screen;

use std::path::Path;
use std::time::Instant;

use overlays_core::screen::{items, samples};

const CASES: &str = "experiments/data/item_slots/cases.json";
const OUT: &str = "experiments/results/items_eval.json";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let samples = samples::load(Path::new(screen::SAMPLES), None).await?.items;
    // 正解は、英語の名前で書いてある
    let named = || {
        samples
            .iter()
            .map(|sample| (sample.name.as_str(), &sample.image))
    };

    // 引数で、ほかの画面（cases.json）を測れる。結果はその隣に書く
    let given = std::env::args().nth(1);
    let out = given
        .as_ref()
        .map(|path| Path::new(path).with_extension("result.json"));
    let cases = screen::cases(given.as_deref().unwrap_or(CASES))?;
    let mut results = Vec::new();
    let mut badges = 0;
    for (case, image) in &cases {
        let scale = case.scale();
        let templates = items::templates(named(), scale);
        let started = Instant::now();
        let slots = items::read(&case.frame(image), &templates, scale, None);
        let elapsed = started.elapsed();
        // 同じ画面をもう一度、前の答えを渡して読む。答えは変わらず、速くなる
        let started = Instant::now();
        let again = items::read(&case.frame(image), &templates, scale, Some(&slots));
        let answers = |slots: &items::Slots| -> Vec<Option<String>> {
            (slots.reads.iter())
                .map(|read| read.answer(items::THRESHOLD).map(str::to_owned))
                .collect()
        };
        anyhow::ensure!(
            answers(&again) == answers(&slots) && again.shop == slots.shop,
            "{}: 前の答えを渡すと、答えが変わった",
            case.file
        );
        println!("  2 回目 {:.1?}", started.elapsed());
        // 段の印で、アイテムの入っている枠が分かる（読めない枠も含めて）。入っている枠を空と読むと、
        // 持っているものを売ったと決めてしまうので、1 つもあってはいけない。逆は、売ったと決めるのが遅れるだけ
        for (slot, (occupied, name)) in slots.occupied.iter().zip(&case.names).enumerate() {
            anyhow::ensure!(
                *occupied || name.is_none(),
                "{}: {} つ目の枠は入っているのに、段の印が読めない",
                case.file,
                slot + 1
            );
            if *occupied && name.is_none() {
                badges += 1;
                println!("  {} つ目の空の枠に、段の印があると読んだ", slot + 1);
            }
        }
        results.push((case, slots.reads, elapsed));
    }
    println!("空の枠に段の印があると読んだ {badges}");
    screen::report(
        out.as_deref().unwrap_or(Path::new(OUT)),
        items::THRESHOLD,
        samples.len(),
        &results,
    )
}
