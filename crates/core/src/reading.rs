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

/// 字幕の 1 語。バイトの位置は元の文のもの
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
        morphemes
            .iter_mut()
            .map(|m| {
                let reading = m
                    .details()
                    .get(READING)
                    .filter(|r| **r != "*")
                    .map(|r| to_hiragana(r));
                let surface = to_hiragana(&m.surface);
                Morpheme {
                    start: m.byte_start,
                    end: m.byte_end,
                    reading: reading.or_else(|| surface.chars().all(is_kana).then_some(surface)),
                }
            })
            .collect()
    }
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
