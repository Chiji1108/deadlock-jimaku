//! Chrome が認識した文のふりがな（DESIGN.md 9.6）。読みは形態素解析（Lindera + IPADIC）で決める。
//! 文脈で決まる読みや造語は誤るが、速く、OpenAI も要らない。ふりがなを LLM にしていれば、確定の文で文に合わない読みを直させる（`furigana.rs`）

use std::borrow::Cow;
use std::sync::OnceLock;

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;

use crate::types::RubySegment as Part;

/// IPADIC の素性のうち「読み」の位置
const READING: usize = 7;

/// 分け方の候補を見る数。20 で、辞書が間違える読みの 9 割に正しい読みが入る（DESIGN.md 6.4）
const PATHS: usize = 20;
/// 1 か所の読みの候補の上限
const MAX_OPTIONS: usize = 6;

/// 字幕の 1 語。バイトの位置は元の文のもの
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Morpheme {
    pub start: usize,
    pub end: usize,
    /// ひらがなの読み。分からなければ None
    pub reading: Option<String>,
}

pub struct Furigana {
    segmenter: Segmenter,
}

impl Furigana {
    /// アプリで 1 つだけ作る辞書。埋め込みの辞書なので、読めなければ None（文をそのまま使う）
    pub fn shared() -> Option<&'static Self> {
        static FURIGANA: OnceLock<Option<Furigana>> = OnceLock::new();
        FURIGANA
            .get_or_init(|| {
                Self::new()
                    .inspect_err(|e| tracing::warn!("furigana: {e:#}"))
                    .ok()
            })
            .as_ref()
    }

    pub fn new() -> anyhow::Result<Self> {
        let dictionary = load_dictionary("embedded://ipadic")?;
        Ok(Self {
            segmenter: Segmenter::new(Mode::Normal, dictionary, None),
        })
    }

    /// 文全体のひらがなの読み。読めない形態素があれば None
    pub fn reading(&self, text: &str) -> Option<String> {
        self.morphemes(text)
            .into_iter()
            .map(|m| m.reading)
            .collect()
    }

    pub fn morphemes(&self, text: &str) -> Vec<Morpheme> {
        let Ok(mut morphemes) = self.segmenter.segment(Cow::Borrowed(text)) else {
            return Vec::new();
        };
        morphemes.iter_mut().map(morpheme).collect()
    }

    /// 辞書のいちばん良い分け方と、読みが分かれるところ（漢字を含む範囲だけ）。
    /// 上位の分け方のうち、いちばん良い分け方と読みが違う範囲を集め、重なる範囲は 1 つにまとめる
    pub fn alternatives(&self, text: &str) -> (Vec<Morpheme>, Vec<Alternatives>) {
        let paths = self
            .segmenter
            .segment_nbest(Cow::Borrowed(text), PATHS, false, None)
            .unwrap_or_default();
        let mut paths: Vec<Vec<Morpheme>> = paths
            .into_iter()
            .map(|(mut tokens, _)| tokens.iter_mut().map(morpheme).collect())
            .collect();
        if paths.is_empty() {
            return (self.morphemes(text), Vec::new());
        }
        let best = paths.remove(0);
        let mut differing: Vec<(usize, usize)> = paths
            .iter()
            .flat_map(|path| differences(text, &best, path))
            .collect();
        differing.sort_unstable();
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for (start, end) in differing {
            match spans.last_mut() {
                Some(last) if start < last.1 => last.1 = last.1.max(end),
                _ => spans.push((start, end)),
            }
        }
        let alternatives = spans.into_iter().filter_map(|(start, end)| {
            let mut options: Vec<Vec<Morpheme>> = Vec::new();
            for path in std::iter::once(&best).chain(&paths) {
                let Some(option) = within(path, start, end) else {
                    continue;
                };
                // 辞書の語をさらに割った分け方は使わない（「火力」を「火」「力」に割ると「ひりょく」と読む）。
                // 同じ語の別の読み（辛い）と、語をまとめた分け方（「一」「人」を「一人」）だけを候補にする
                let splits = option[1..]
                    .iter()
                    .any(|m| best.iter().all(|b| b.start != m.start));
                let read = read(text, option);
                if !splits
                    && options.len() < MAX_OPTIONS
                    && options.iter().all(|o| read != self::read(text, o))
                {
                    options.push(option.to_vec());
                }
            }
            let kanji = text[start..end].chars().any(is_kanji);
            (kanji && options.len() > 1).then_some(Alternatives {
                start,
                end,
                options,
            })
        });
        let alternatives = alternatives.collect();
        (best, alternatives)
    }
}

/// 読みが分かれるところ。`options` の先頭が、辞書のいちばん良い読み
#[derive(Debug)]
pub struct Alternatives {
    pub start: usize,
    pub end: usize,
    pub options: Vec<Vec<Morpheme>>,
}

fn morpheme(token: &mut lindera::token::Token) -> Morpheme {
    let reading = token
        .details()
        .get(READING)
        .filter(|r| **r != "*")
        .map(|r| to_hiragana(r));
    let surface = to_hiragana(&token.surface);
    Morpheme {
        start: token.byte_start,
        end: token.byte_end,
        reading: reading.or_else(|| surface.chars().all(is_kana).then_some(surface)),
    }
}

/// `start..end` をちょうど覆う語。この分け方の語の境目に合わなければ None
fn within(path: &[Morpheme], start: usize, end: usize) -> Option<&[Morpheme]> {
    let from = path.iter().position(|m| m.start == start)?;
    let to = path.iter().position(|m| m.end == end)?;
    path.get(from..=to)
}

/// 語をつなげた読み。読めない語は、文字のまま
pub fn read(text: &str, morphemes: &[Morpheme]) -> String {
    morphemes
        .iter()
        .map(|m| m.reading.as_deref().unwrap_or(&text[m.start..m.end]))
        .collect()
}

/// 2 つの分け方で、読みが違う範囲。両方の語の境目になっている位置で区切って比べる
fn differences(text: &str, best: &[Morpheme], path: &[Morpheme]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let (mut b, mut p, mut start) = (0, 0, 0);
    let (mut from_b, mut from_p) = (0, 0);
    while b < best.len() && p < path.len() {
        match best[b].end.cmp(&path[p].end) {
            std::cmp::Ordering::Less => b += 1,
            std::cmp::Ordering::Greater => p += 1,
            std::cmp::Ordering::Equal => {
                let end = best[b].end;
                (b, p) = (b + 1, p + 1);
                if read(text, &best[from_b..b]) != read(text, &path[from_p..p]) {
                    out.push((start, end));
                }
                (from_b, from_p, start) = (b, p, end);
            }
        }
    }
    out
}

/// 文の `from..to` にふりがなを付ける。形態素にならなかった文字（空白など）もそのまま残す
pub fn annotate(text: &str, morphemes: &[Morpheme], from: usize, to: usize) -> Vec<Part> {
    let mut out: Vec<Part> = Vec::new();
    let mut pos = from;
    for m in morphemes.iter().filter(|m| from <= m.start && m.end <= to) {
        if pos < m.start {
            push(&mut out, plain(&text[pos..m.start]));
        }
        for part in ruby_tokens(&text[m.start..m.end], m.reading.as_deref()) {
            push(&mut out, part);
        }
        pos = m.end;
    }
    if pos < to {
        push(&mut out, plain(&text[pos..to]));
    }
    out
}

/// ルビのない文字は前の部分につなげる
fn push(out: &mut Vec<Part>, part: Part) {
    if let (
        Some(Part {
            text: prev,
            ruby: None,
        }),
        None,
    ) = (out.last_mut(), &part.ruby)
    {
        prev.push_str(&part.text);
        return;
    }
    out.push(part);
}

fn plain(text: &str) -> Part {
    Part {
        text: text.into(),
        ruby: None,
    }
}

/// 語の読みを漢字の部分ごとに割り当てる。漢字のない語や、かな以外の読みにはルビを付けない。
/// 「強い / つよい」→「強(つよ)」「い」、「取り扱い / とりあつかい」→「取(と)」「り」「扱(あつか)」「い」
fn ruby_tokens(text: &str, ruby: Option<&str>) -> Vec<Part> {
    let Some(ruby) = ruby.filter(|r| !r.is_empty() && r.chars().all(is_kana)) else {
        return vec![plain(text)];
    };
    if !text.chars().any(is_kanji) {
        return vec![plain(text)];
    }
    align(text, &to_hiragana(ruby)).unwrap_or_else(|| vec![plain(text)])
}

/// 漢字の続く部分と、それ以外（かななど）の部分に分け、かなの部分を読みの中で探して区切る。
/// 読みとかなの部分が合わなければ None
pub fn align(text: &str, reading: &str) -> Option<Vec<Part>> {
    let mut runs: Vec<(bool, String)> = Vec::new();
    for c in text.chars() {
        match runs.last_mut() {
            Some((kanji, run)) if *kanji == is_kanji(c) => run.push(c),
            _ => runs.push((is_kanji(c), c.to_string())),
        }
    }
    let r: Vec<char> = reading.chars().collect();
    let mut pos = 0;
    let mut out = Vec::new();
    for (i, (kanji, run)) in runs.iter().enumerate() {
        if !kanji {
            let kana: Vec<char> = to_hiragana(run).chars().collect();
            if r.get(pos..pos + kana.len())? != kana.as_slice() {
                return None;
            }
            out.push(plain(run));
            pos += kana.len();
            continue;
        }
        // 漢字の読みは、次のかなの部分が読みに現れるところまで（漢字 1 文字に少なくとも 1 文字）
        let end = match runs.get(i + 1) {
            None => r.len(),
            Some((_, next)) => {
                let next: Vec<char> = to_hiragana(next).chars().collect();
                (pos + 1..=r.len().saturating_sub(next.len()))
                    .find(|&j| r[j..].starts_with(&next))?
            }
        };
        if end <= pos {
            return None;
        }
        out.push(Part {
            text: run.clone(),
            ruby: Some(r[pos..end].iter().collect()),
        });
        pos = end;
    }
    (pos == r.len()).then_some(out)
}

pub fn to_hiragana(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            // カタカナ（ァ〜ヶ）をひらがなに。長音符などはそのまま
            '\u{30A1}'..='\u{30F6}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

pub fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}')
}

fn is_kanji(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '々')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 読みが分かれるところの、文字と読みの候補
    fn alternatives(text: &str) -> Vec<(String, Vec<String>)> {
        let (_, found) = Furigana::shared().unwrap().alternatives(text);
        let listed = found.iter().map(|a| {
            let options = a.options.iter().map(|o| read(text, o)).collect();
            (text[a.start..a.end].to_owned(), options)
        });
        listed.collect()
    }

    #[test]
    fn alternatives_are_other_readings_of_the_same_words() {
        let of = |text: &str, word: &str| {
            let found = alternatives(text);
            let (_, options) = found.into_iter().find(|(w, _)| w == word).expect(word);
            options
        };
        // 先頭が、辞書のいちばん良い読み
        let spicy = of("このカレー辛い", "辛い");
        assert_eq!(spicy[0], "つらい");
        assert!(spicy.contains(&"からい".to_owned()));
        // 語をまとめた分け方（「一」「人」を「一人」）も候補にする
        assert!(of("一人で倒した", "一人").contains(&"ひとり".to_owned()));
        // 辞書の語をさらに割った読み（「火」「力」の「ひりょく」）は候補にしない
        let all = alternatives("火力が上がってる");
        assert!(
            all.iter()
                .all(|(_, options)| !options.iter().any(|o| o.contains("ひりょく")))
        );
        // かなだけのところは聞かない
        assert!(alternatives("めっちゃつよい").is_empty());
    }

    fn p(text: &str, ruby: Option<&str>) -> Part {
        Part {
            text: text.into(),
            ruby: ruby.map(Into::into),
        }
    }

    #[test]
    fn ruby_goes_only_on_kanji() {
        assert_eq!(
            ruby_tokens("強い", Some("つよい")),
            [p("強", Some("つよ")), p("い", None)]
        );
        assert_eq!(
            ruby_tokens("取り扱い", Some("とりあつかい")),
            [
                p("取", Some("と")),
                p("り", None),
                p("扱", Some("あつか")),
                p("い", None)
            ]
        );
    }

    #[test]
    fn reads_a_sentence_with_the_dictionary() {
        let furigana = Furigana::new().unwrap();
        let text = "実況です";
        let parts = annotate(text, &furigana.morphemes(text), 0, text.len());
        assert_eq!(parts, [p("実況", Some("じっきょう")), p("です", None)]);
    }

    #[test]
    fn spaces_stay_in_the_text() {
        let furigana = Furigana::new().unwrap();
        let text = "Deadlock is fun";
        let parts = annotate(text, &furigana.morphemes(text), 0, text.len());
        assert_eq!(parts, [p(text, None)]);
    }
}
