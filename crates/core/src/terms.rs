//! LLM を使わない用語の判定（DESIGN.md 9.6）。形態素の区切りに沿って、書き方か読みが公式名と
//! 同じところを探す。あいまいな一致はしない

use std::collections::HashMap;

use crate::reading::{Morpheme, align, is_kana, to_hiragana};
use crate::types::RubySegment as Part;

/// これより短い読みは比べない。公式名と違う書き方の言葉に当たる（「皆」がミナ、「山と」がヤマト）
const MIN_READING: usize = 4;
/// 名前は長くてもこの数の形態素に収まる
const MAX_MORPHEMES: usize = 12;

/// 辞書の 1 つの名前。アプリが辞書から作る
#[derive(Debug, PartialEq)]
pub struct Term {
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
                by_reading.entry(fold(&reading)).or_insert(i);
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
            let hit = (i + 1..=longest).rev().find_map(|end| {
                let run = &morphemes[i..end];
                let whole = whole(text, run[0].start, run[run.len() - 1].end);
                Some((end, self.lookup(text, run).filter(|_| whole)?))
            });
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
            self.by_reading.get(&fold(&reading)).copied()
        })
    }
}

/// 読みを比べる前に、同じ音の書き方の揺れをそろえる（ひらがなの読みに使う）。文字起こしは、公式名と同じ音を
/// 別の字で書く（DESIGN.md 3 章の合成音声の測定）: 「ヴィスカス」を「ビスカス」、「ホリデイ」を「ホリデー」、
/// 「レディガイスト」を「レディーガイスト」。ヴの行はバの行に、長音符は落とし、エの段のあとの「い」も落とす
fn fold(reading: &str) -> String {
    let mut folded = String::with_capacity(reading.len());
    let mut chars = reading.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            'ゔ' => folded.push(match chars.next_if(|next| "ぁぃぇぉ".contains(*next)) {
                Some('ぁ') => 'ば',
                Some('ぃ') => 'び',
                Some('ぇ') => 'べ',
                Some('ぉ') => 'ぼ',
                _ => 'ぶ',
            }),
            'ー' => {}
            'い' if folded.ends_with(E_ROW) => {}
            _ => folded.push(c),
        }
    }
    folded
}

/// エの段のかな（あとの「い」は、長音として書かれることがある）
const E_ROW: [char; 15] = [
    'え', 'け', 'せ', 'て', 'ね', 'へ', 'め', 'れ', 'げ', 'ぜ', 'で', 'べ', 'ぺ', 'ぇ', 'ゑ',
];

/// `start..end` が、カタカナの語の途中で切れていない。辞書は知らないカタカナの語を知っている語に割ることが
/// あるので（「レムソン」を「レム」「ソン」）、カタカナが続く途中では用語にしない
fn whole(text: &str, start: usize, end: usize) -> bool {
    let katakana = |c: char| matches!(c, 'ァ'..='ヴ' | 'ー');
    let joins =
        |a: Option<char>, b: Option<char>| a.is_some_and(katakana) && b.is_some_and(katakana);
    let (before, word, after) = (&text[..start], &text[start..end], &text[end..]);
    !joins(before.chars().next_back(), word.chars().next())
        && !joins(word.chars().next_back(), after.chars().next())
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
    use crate::reading::Furigana;

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
            term("hero:35", "ヴィスカス", "ヴィスカス", "Viscous"),
            term("hero:14", "ホリデイ", "ホリデイ", "Holliday"),
            term("hero:4", "レディガイスト", "レディガイスト", "Lady Geist"),
            term("hero:3", "ヴィンディクタ", "ヴィンディクタ", "Vindicta"),
        ])
    }

    /// 見つけた用語の（話した言葉、参照）
    fn find(sentence: &str) -> Vec<(String, String)> {
        let furigana = Furigana::shared().unwrap();
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
    fn the_same_sound_in_another_spelling_matches() {
        assert_eq!(find("ビスカスのパンチ痛い"), found("ビスカス", "hero:35"));
        assert_eq!(find("ホリデーが飛んできた"), found("ホリデー", "hero:14"));
        assert_eq!(
            find("レディーガイストが近い"),
            found("レディーガイスト", "hero:4")
        );
        assert_eq!(
            find("ビンディクタが飛んでる"),
            found("ビンディクタ", "hero:3")
        );
        // そろえるのは音が同じ書き方だけ。違う音は別の言葉のまま
        assert_eq!(find("ビスカが痛い"), Vec::new());
        assert_eq!(fold("ゔぁいぱー"), "ばいぱ");
        assert_eq!(fold("ぺいじ"), fold("ぺーじ"));
        assert_eq!(fold("けいさん"), "けさん");
        assert_eq!(fold("あいす"), "あいす");
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
        let furigana = Furigana::shared().unwrap();
        let sentence = "ヘイズのカース";
        let terms = Terms::new(Vec::new());
        assert!(
            terms
                .find(sentence, &furigana.morphemes(sentence))
                .is_empty()
        );
    }
}
