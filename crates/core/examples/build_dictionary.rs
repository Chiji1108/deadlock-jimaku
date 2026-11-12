//! 読みの辞書（`crates/core/dictionary/`）を、国立国語研究所の「現代話し言葉 UniDic」から組み立てる（DESIGN.md 6.4）。
//!
//! 配布物は MeCab 用で、語のつながりやすさの表が 1 GB ある。vibrato の小さい形（表を、もとになった規則と重みで持つ）に
//! すると 74 MB（圧縮して 12 MB）になり、アプリに同梱できる。素性は品詞と読み（仮名形出現形）だけを残す。
//! 同じ文字で読みが 2 つ以上ある語の表（`homographs.tsv.gz`）も作る（ふりがなの読みを判定モデルに選ばせる候補）。
//!
//! usage: cargo run --release -p overlays-core --example build_dictionary -- SRC_DIR
//!   SRC_DIR は `unidic-csj-202512_full.zip`（https://clrd.ninjal.ac.jp/unidic/back_number.html#unidic_csj、3 GB）を
//!   広げたフォルダ。`lex.csv`、`unk.def`、`char.def`、`feature.def`、`left-id.def`、`right-id.def`、`model.def` を読む

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;
use overlays_core::reading::{is_kana, is_kanji, to_hiragana};
use vibrato::SystemDictionaryBuilder;

/// UniDic の素性のうち、仮名形出現形の位置
const KANA: usize = 20;
/// 品詞の中分類と、語頭変化形（基本形、濁音形）の位置
const POS2: usize = 1;
const INITIAL_FORM: usize = 14;
/// いちばんよく使う読みより、語のコストがこれより大きい読みは、候補にしない。「人気」の「ひとけ」（+4916）は残り、
/// 「大会」の「だいえ」（+5713）と「何」の「なぬ」（+7057）は外れる
const RARE: i32 = 5000;
/// MeCab の `dicrc` の `cost-factor`
const COST_FACTOR: f64 = 700.0;
const OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/dictionary");

/// CSV の 1 行（引用符の中のカンマに対応する）
fn row(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                fields.last_mut().expect("a field").push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(String::new()),
            c => fields.last_mut().expect("a field").push(c),
        }
    }
    fields
}

fn quote(field: &str) -> String {
    if field.contains([',', '"']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

/// 語の行（文字、左の ID、右の ID、コスト、素性…）
fn rows(path: &Path) -> anyhow::Result<Vec<Vec<String>>> {
    let lines = BufReader::new(File::open(path)?).lines();
    let rows = lines.map(|line| Ok(row(&line?)));
    let rows: anyhow::Result<Vec<_>> = rows.collect();
    Ok(rows?.into_iter().filter(|row| row.len() > 4).collect())
}

/// 素性を、品詞と読みだけにする。語のつながりやすさは ID で決まるので、解析の結果は変わらない
fn slim(rows: &[Vec<String>]) -> Vec<u8> {
    let mut out = Vec::new();
    for row in rows {
        let kana = row.get(4 + KANA).map_or("*", String::as_str);
        let (word, pos) = (quote(&row[0]), quote(&row[4]));
        let (left, right, cost) = (&row[1], &row[2], &row[3]);
        writeln!(out, "{word},{left},{right},{cost},{pos},{}", quote(kana)).expect("write");
    }
    out
}

/// 同じ文字で読みが 2 つ以上ある、漢字を含む語。読みは、よく使うもの（コストが小さいもの）から
fn homographs(rows: &[Vec<String>]) -> Vec<u8> {
    let mut readings: BTreeMap<&str, Vec<(i32, String)>> = BTreeMap::new();
    for row in rows {
        let (word, kana) = (
            row[0].as_str(),
            row.get(4 + KANA).map_or("", String::as_str),
        );
        if !word.chars().any(is_kanji) || kana.is_empty() || !kana.chars().all(is_kana) {
            continue;
        }
        // 複合語の中でだけ出る濁った読み（「辛い」の「がらい」）と、固有名詞の読み（「金」の「きむ」）は候補にしない
        let (kind, initial) = (row.get(4 + POS2), row.get(4 + INITIAL_FORM));
        if kind.is_some_and(|k| k == "固有名詞") || initial.is_some_and(|i| i.contains("濁音形"))
        {
            continue;
        }
        let cost: i32 = row[3].parse().unwrap_or(i32::MAX);
        let reading = to_hiragana(kana);
        let known = readings.entry(word).or_default();
        match known.iter_mut().find(|(_, r)| *r == reading) {
            Some(same) => same.0 = same.0.min(cost),
            None => known.push((cost, reading)),
        }
    }
    let mut out = Vec::new();
    for (word, mut known) in readings {
        known.sort();
        // めったに使わない読み（「何」の「なぬ」、「大会」の「だいえ」）は候補にしない
        let common = known[0].0.saturating_add(RARE);
        known.retain(|(cost, _)| *cost <= common);
        if known.len() < 2 {
            continue;
        }
        let known: Vec<String> = known.into_iter().map(|(_, reading)| reading).collect();
        writeln!(out, "{word}\t{}", known.join(",")).expect("write");
    }
    out
}

fn gzip(path: &str, bytes: &[u8]) -> anyhow::Result<u64> {
    let mut encoder = GzEncoder::new(File::create(path)?, Compression::best());
    encoder.write_all(bytes)?;
    encoder.finish()?;
    Ok(std::fs::metadata(path)?.len())
}

fn main() -> anyhow::Result<()> {
    let src = std::env::args().nth(1).expect("SRC_DIR");
    let src = Path::new(&src);
    let lexicon = rows(&src.join("lex.csv"))?;
    let unknown = rows(&src.join("unk.def"))?;
    // 語のつながりやすさを、表（matrix.def、8.5 GB）ではなく、規則（feature.def）と重み（model.def）から作る
    let (mut right, mut left, mut cost) = (Vec::new(), Vec::new(), Vec::new());
    vibrato::mecab::generate_bigram_info(
        File::open(src.join("feature.def"))?,
        File::open(src.join("right-id.def"))?,
        File::open(src.join("left-id.def"))?,
        File::open(src.join("model.def"))?,
        COST_FACTOR,
        &mut right,
        &mut left,
        &mut cost,
    )?;
    let dictionary = SystemDictionaryBuilder::from_readers_with_bigram_info(
        Cursor::new(slim(&lexicon)),
        Cursor::new(right),
        Cursor::new(left),
        Cursor::new(cost),
        File::open(src.join("char.def"))?,
        Cursor::new(slim(&unknown)),
        false,
    )?;
    let mut raw = Vec::new();
    dictionary.write(&mut raw)?;
    let size = gzip(&format!("{OUT}/unidic-csj.dic.gz"), &raw)?;
    println!(
        "dictionary: {} MB, {} MB compressed",
        raw.len() >> 20,
        size >> 20
    );
    let table = homographs(&lexicon);
    let size = gzip(&format!("{OUT}/homographs.tsv.gz"), &table)?;
    let words = table.iter().filter(|b| **b == b'\n').count();
    println!("homographs: {words} words, {} KB compressed", size >> 10);
    Ok(())
}
