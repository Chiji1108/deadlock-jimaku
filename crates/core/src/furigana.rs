//! ふりがな（DESIGN.md 6.4）。読みは形態素解析（Lindera + IPADIC）で決める。
//!
//! LLM に読みを付けさせると、ずれたり（僕→ぼ）、送り仮名ごと付けたり（強い→つよい）するので使わない。

use std::borrow::Cow;

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;

use crate::types::Token;

/// IPADIC の素性のうち「読み」の位置
const READING: usize = 7;

pub struct Furigana {
    segmenter: Segmenter,
}

impl Furigana {
    /// 辞書の読み込みに少し時間がかかるので、起動時に 1 回だけ作る
    pub fn new() -> anyhow::Result<Self> {
        let dictionary = load_dictionary("embedded://ipadic")?;
        Ok(Self {
            segmenter: Segmenter::new(Mode::Normal, dictionary, None),
        })
    }

    /// 漢字を含む語にルビを付けたトークン列にする（かなだけの部分はまとめる）
    pub fn annotate(&self, text: &str) -> Vec<Token> {
        let mut out: Vec<Token> = Vec::new();
        let Ok(mut morphemes) = self.segmenter.segment(Cow::Borrowed(text)) else {
            return vec![plain(text)];
        };
        for m in morphemes.iter_mut() {
            let surface = m.surface.to_string();
            let reading = m
                .details()
                .get(READING)
                .filter(|r| **r != "*")
                .map(|r| to_hiragana(r));
            for token in ruby_tokens(&surface, reading.as_deref()) {
                push(&mut out, token);
            }
        }
        out
    }
}

/// ルビのない文字は前のトークンにつなげる
fn push(out: &mut Vec<Token>, token: Token) {
    if let (
        Some(Token::Text {
            text: prev,
            ruby: None,
        }),
        Token::Text { text, ruby: None },
    ) = (out.last_mut(), &token)
    {
        prev.push_str(text);
        return;
    }
    out.push(token);
}

fn plain(text: &str) -> Token {
    Token::Text {
        text: text.into(),
        ruby: None,
    }
}

/// ルビを確かめて、送り仮名を外に出す。
/// 漢字のない語や、かな以外の読みにはルビを付けない。
/// 「強い / つよい」→「強(つよ)」「い」、「お金 / おかね」→「お」「金(かね)」
pub fn ruby_tokens(text: &str, ruby: Option<&str>) -> Vec<Token> {
    let Some(ruby) = ruby.filter(|r| !r.is_empty() && r.chars().all(is_kana)) else {
        return vec![plain(text)];
    };
    if !text.chars().any(is_kanji) {
        return vec![plain(text)];
    }
    let t: Vec<char> = text.chars().collect();
    let r: Vec<char> = ruby.chars().collect();
    let prefix = t
        .iter()
        .zip(&r)
        .take_while(|(a, b)| a == b && is_kana(**a))
        .count();
    let suffix = t[prefix..]
        .iter()
        .rev()
        .zip(r[prefix..].iter().rev())
        .take_while(|(a, b)| a == b && is_kana(**a))
        .count();
    let core: String = t[prefix..t.len() - suffix].iter().collect();
    let core_ruby: String = r[prefix..r.len() - suffix].iter().collect();
    if core.is_empty() || core_ruby.is_empty() {
        return vec![plain(text)];
    }
    let head: String = t[..prefix].iter().collect();
    let tail: String = t[t.len() - suffix..].iter().collect();
    [
        (!head.is_empty()).then(|| plain(&head)),
        Some(Token::Text {
            text: core,
            ruby: Some(core_ruby),
        }),
        (!tail.is_empty()).then(|| plain(&tail)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn to_hiragana(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            // カタカナ（ァ〜ヶ）をひらがなに。長音符などはそのまま
            '\u{30A1}'..='\u{30F6}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}')
}

fn is_kanji(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '々')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(text: &str, ruby: Option<&str>) -> Token {
        Token::Text {
            text: text.into(),
            ruby: ruby.map(Into::into),
        }
    }

    #[test]
    fn ruby_moves_okurigana_and_prefix_out() {
        assert_eq!(
            ruby_tokens("強い", Some("つよい")),
            [t("強", Some("つよ")), t("い", None)]
        );
        assert_eq!(
            ruby_tokens("お金", Some("おかね")),
            [t("お", None), t("金", Some("かね"))]
        );
        assert_eq!(
            ruby_tokens("実況", Some("じっきょう")),
            [t("実況", Some("じっきょう"))]
        );
    }

    #[test]
    fn ruby_is_dropped_when_wrong() {
        assert_eq!(ruby_tokens("ヘイズ", Some("へいず")), [t("ヘイズ", None)]);
        assert_eq!(ruby_tokens("強い", Some("tsuyoi")), [t("強い", None)]);
        assert_eq!(ruby_tokens("つよい", Some("つよい")), [t("つよい", None)]);
    }

    #[test]
    fn annotates_sentence_with_readings() {
        let f = Furigana::new().unwrap();
        assert_eq!(
            f.annotate("僕の一番好きなキャラ"),
            [
                t("僕", Some("ぼく")),
                t("の", None),
                t("一番", Some("いちばん")),
                t("好", Some("す")),
                t("きなキャラ", None),
            ]
        );
        assert_eq!(
            f.annotate("いや、楽しみだな。"),
            [
                t("いや、", None),
                t("楽", Some("たの")),
                t("しみだな。", None)
            ]
        );
    }
}
