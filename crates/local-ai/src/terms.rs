//! LLM を使わない用語の判定（DESIGN.md 9.6）。辞書の名前と別名を、形態素の区切りに沿って探す。
//!
//! 書き方の一致を先に見て、次に読みの一致を見る。あいまいな一致はしない。ふつうの言葉を用語に
//! するより、見逃すほうがいいので、短すぎる読みは比べない

use std::collections::HashMap;

use serde::Deserialize;

use crate::furigana::{Morpheme, Part, align, is_kana, to_hiragana};

/// これより短い読みは、ふつうの言葉に当たりやすいので比べない（「かくまが」は 4 文字）
const MIN_READING: usize = 4;
/// 名前は長くてもこの数の形態素に収まる
const MAX_MORPHEMES: usize = 12;

/// 用語の 1 つの書き方（公式名か別名）。アプリが辞書から作る
#[derive(Debug, Deserialize)]
pub struct Term {
    #[serde(rename = "ref")]
    pub reference: String,
    pub text: String,
    /// 読み（名前がかなだけなら名前そのもの）
    pub reading: String,
    pub en: String,
}

/// 字幕の中で見つけた用語。バイトの位置で、形態素の境目に揃っている
pub struct Found<'a> {
    pub start: usize,
    pub end: usize,
    pub term: &'a Term,
}

pub struct Terms {
    terms: Vec<Term>,
    by_text: HashMap<String, usize>,
    by_reading: HashMap<String, usize>,
}

impl Terms {
    pub fn new(terms: Vec<Term>) -> Self {
        let mut by_text = HashMap::new();
        let mut by_reading = HashMap::new();
        for (i, term) in terms.iter().enumerate() {
            by_text.entry(term.text.clone()).or_insert(i);
            let reading = to_hiragana(&term.reading);
            if reading.chars().count() >= MIN_READING && reading.chars().all(is_kana) {
                by_reading.entry(reading).or_insert(i);
            }
        }
        Self {
            terms,
            by_text,
            by_reading,
        }
    }

    /// 重ならない用語を前から探す。同じ位置からは、いちばん長いものを取る
    pub fn find(&self, text: &str, morphemes: &[Morpheme]) -> Vec<Found<'_>> {
        let mut found = Vec::new();
        let mut i = 0;
        while i < morphemes.len() {
            let longest = morphemes.len().min(i + MAX_MORPHEMES);
            let hit = (i + 1..=longest)
                .rev()
                .find_map(|end| Some((end, self.lookup(text, &morphemes[i..end])?)));
            let Some((end, term)) = hit else {
                i += 1;
                continue;
            };
            found.push(Found {
                start: morphemes[i].start,
                end: morphemes[end - 1].end,
                term: &self.terms[term],
            });
            i = end;
        }
        found
    }

    fn lookup(&self, text: &str, run: &[Morpheme]) -> Option<usize> {
        let surface = &text[run.first()?.start..run.last()?.end];
        self.by_text.get(surface).copied().or_else(|| {
            let reading = run
                .iter()
                .map(|m| m.reading.as_deref())
                .collect::<Option<String>>()?;
            self.by_reading.get(&reading).copied()
        })
    }
}

/// 話した言葉のふりがな。用語の読みがかなで、話した言葉に当てはまるときだけ
pub fn ruby(term: &Term, spoken: &str) -> Option<Vec<Part>> {
    let reading = to_hiragana(&term.reading);
    reading
        .chars()
        .all(is_kana)
        .then(|| align(spoken, &reading))
        .flatten()
        .filter(|parts| parts.iter().any(|p| p.ruby.is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::furigana::Furigana;

    fn term(reference: &str, text: &str, reading: &str, en: &str) -> Term {
        Term {
            reference: reference.into(),
            text: text.into(),
            reading: reading.into(),
            en: en.into(),
        }
    }

    fn table() -> Terms {
        Terms::new(vec![
            term("hero:13", "ヘイズ", "ヘイズ", "Haze"),
            term("hero:9", "ペイジ", "ペイジ", "Paige"),
            term(
                "item:1",
                "基礎拡張マガジン",
                "きそかくちょうマガジン",
                "Extended Magazine",
            ),
            term("item:1", "カクマガ", "カクマガ", "Extended Magazine"),
            term("item:1", "角磨が", "角磨が", "Extended Magazine"),
            term("item:2", "呪いの遺物", "のろいのいぶつ", "Cursed Relic"),
            term("item:2", "カース", "カース", "Cursed Relic"),
            term("item:3", "カースブレイカー", "カースブレイカー", "Breaker"),
            term("hero:8", "レム", "レム", "Lemur"),
        ])
    }

    /// 見つけた用語の（話した言葉、参照）
    fn find(sentence: &str) -> Vec<(String, String)> {
        let furigana = Furigana::new().unwrap();
        let terms = table();
        terms
            .find(sentence, &furigana.morphemes(sentence))
            .iter()
            .map(|f| {
                (
                    sentence[f.start..f.end].to_owned(),
                    f.term.reference.clone(),
                )
            })
            .collect()
    }

    fn found(spoken: &str, reference: &str) -> Vec<(String, String)> {
        vec![(spoken.to_owned(), reference.to_owned())]
    }

    #[test]
    fn exact_names_match() {
        assert_eq!(find("ヘイズめっちゃ強い"), found("ヘイズ", "hero:13"));
        assert_eq!(
            find("基礎拡張マガジンを買う"),
            found("基礎拡張マガジン", "item:1")
        );
        assert_eq!(find("カースは最終盤に"), found("カース", "item:2"));
    }

    #[test]
    fn readings_match_other_spellings() {
        assert_eq!(find("呪いの異物だね"), found("呪いの異物", "item:2"));
        assert_eq!(find("かくまがを取りたい"), found("かくまが", "item:1"));
        assert_eq!(find("へいずが強い"), Vec::new(), "3 文字の読みは比べない");
    }

    #[test]
    fn longest_name_wins() {
        assert_eq!(
            find("カースブレイカーを買う"),
            found("カースブレイカー", "item:3")
        );
    }

    #[test]
    fn an_observed_misrecognition_is_an_alias() {
        assert_eq!(find("角磨がをちゃんとね"), found("角磨が", "item:1"));
    }

    #[test]
    fn ordinary_words_stay_plain() {
        for sentence in [
            "次のページを見て",
            "コンビニでカートを押す",
            "レムソンを飲む",
            "角を磨がないと",
            "カーストの話をする",
        ] {
            assert_eq!(find(sentence), Vec::new(), "{sentence}");
        }
    }

    #[test]
    fn no_terms_finds_nothing() {
        let furigana = Furigana::new().unwrap();
        let sentence = "ヘイズのカース";
        let terms = Terms::new(Vec::new());
        assert!(
            terms
                .find(sentence, &furigana.morphemes(sentence))
                .is_empty()
        );
    }
}
