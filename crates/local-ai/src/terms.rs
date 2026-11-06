//! LLM を使わない用語の判定（DESIGN.md 9.6）。形態素の区切りに沿って、書き方か読みが公式名と
//! 同じところを探す。あいまいな一致はしない

use std::collections::HashMap;

use serde::Deserialize;

use crate::furigana::{Morpheme, Part, align, is_kana, to_hiragana};

/// これより短い読みは比べない。公式名と違う書き方の言葉に当たる（「皆」がミナ、「山と」がヤマト）
const MIN_READING: usize = 4;
/// 名前は長くてもこの数の形態素に収まる
const MAX_MORPHEMES: usize = 12;

/// 辞書の 1 つの名前。アプリが辞書から作る
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
            term("hero:50", "ポケット", "ポケット", "Pocket"),
            term("hero:52", "ミナ", "ミナ", "Mina"),
            term("hero:17", "ヤマト", "ヤマト", "Yamato"),
            term(
                "item:1",
                "基礎拡張マガジン",
                "きそかくちょうマガジン",
                "Extended Magazine",
            ),
            term("item:2", "呪いの遺物", "のろいのいぶつ", "Cursed Relic"),
            term("item:3", "カースブレイカー", "カースブレイカー", "Breaker"),
            term("ability:1", "カース", "カース", "Curse"),
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
    }

    #[test]
    fn an_ordinary_word_that_equals_a_name_matches() {
        assert_eq!(find("ポケットに入れる"), found("ポケット", "hero:50"));
    }

    #[test]
    fn readings_match_other_spellings() {
        assert_eq!(find("呪いの異物だね"), found("呪いの異物", "item:2"));
        assert_eq!(
            find("きそかくちょうまがじんを取る"),
            found("きそかくちょうまがじん", "item:1")
        );
    }

    #[test]
    fn short_readings_are_not_compared() {
        for sentence in ["皆で行こう", "山と谷がある", "よく見な", "へいずが強い"]
        {
            assert_eq!(find(sentence), Vec::new(), "{sentence}");
        }
    }

    #[test]
    fn longest_name_wins() {
        assert_eq!(
            find("カースブレイカーを買う"),
            found("カースブレイカー", "item:3")
        );
    }

    #[test]
    fn words_that_are_not_official_names_stay_plain() {
        for sentence in [
            "カクマガを取りたい",
            "角磨がをちゃんとね",
            "次のページを見て",
            "レムソンを飲む",
            "カーストの話をする",
            "ヘイズウルトを使う",
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
