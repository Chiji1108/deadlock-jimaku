//! ふりがな（DESIGN.md 6.4）。読みは形態素解析（Lindera + IPADIC）で決める。
//!
//! 字幕の文は毎回ここで読む（LLM は読みがずれたり、別の文を作ったりするので使わない）。
//! 辞書の漢字名は造語が多いので、LLM の読みを重ねる（`assets::readings`。検証にはここの関数を使う）。

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

/// 語の読みを漢字の部分ごとに割り当てる。漢字のない語や、かな以外の読みにはルビを付けない。
/// 「強い / つよい」→「強(つよ)」「い」、「取り扱い / とりあつかい」→「取(と)」「り」「扱(あつか)」「い」
pub fn ruby_tokens(text: &str, ruby: Option<&str>) -> Vec<Token> {
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
pub fn align(text: &str, reading: &str) -> Option<Vec<Token>> {
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
        out.push(Token::Text {
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

fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}')
}

pub fn is_kanji(c: char) -> bool {
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
    fn ruby_is_split_around_kana_inside_a_word() {
        assert_eq!(
            ruby_tokens("取り扱い", Some("とりあつかい")),
            [
                t("取", Some("と")),
                t("り", None),
                t("扱", Some("あつか")),
                t("い", None)
            ]
        );
        assert_eq!(
            align("魔呪の罠", "まじゅのわな"),
            Some(vec![
                t("魔呪", Some("まじゅ")),
                t("の", None),
                t("罠", Some("わな"))
            ])
        );
        // かなの部分が読みに現れない
        assert_eq!(align("取り扱い", "とあつかい"), None);
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
