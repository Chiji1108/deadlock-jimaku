//! 短い記法から Token の列を作る。
//!
//! - `{hero:13}` / `{hero:13|ヘイズ}`: エンティティ（`|` の後は発話された表現）
//! - `[漢字|かんじ]`: ルビ（`parse` だけ。LLM の訳文には使わない）
//!
//! 閉じていない括弧はそのまま文字として扱う（LLM の出力を落とさない）。

use crate::types::Token;

/// ルビとエンティティの両方を読む（ダミー字幕用）
pub fn parse(markup: &str) -> Vec<Token> {
    parse_with(markup, &['[', '{'])
}

/// エンティティだけを読む（LLM の訳文用。`[` は普通の文字として残す）
pub fn parse_entities(markup: &str) -> Vec<Token> {
    parse_with(markup, &['{'])
}

fn parse_with(markup: &str, opens: &[char]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut rest = markup;
    while let Some(start) = rest.find(opens) {
        let ruby = rest[start..].starts_with('[');
        let close = if ruby { ']' } else { '}' };
        let Some(len) = rest[start..].find(close) else {
            break;
        };
        let end = start + len;
        push_text(&mut tokens, &rest[..start], None);
        let inner = &rest[start + 1..end];
        let (head, tail) = inner
            .split_once('|')
            .map_or((inner, None), |(h, t)| (h, Some(t)));
        if ruby {
            push_text(&mut tokens, head, tail);
        } else {
            tokens.push(Token::Entity {
                reference: head.trim().into(),
                spoken: tail.map(Into::into),
            });
        }
        rest = &rest[end + 1..];
    }
    push_text(&mut tokens, rest, None);
    tokens
}

fn push_text(tokens: &mut Vec<Token>, text: &str, ruby: Option<&str>) {
    if !text.is_empty() {
        tokens.push(Token::Text {
            text: text.into(),
            ruby: ruby.map(Into::into),
        });
    }
}

/// 音声認識の結果に相当する文字列（エンティティは発話された表現）
pub fn plain(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|t| match t {
            Token::Text { text, .. } => text.as_str(),
            Token::Entity { spoken, reference } => spoken.as_deref().unwrap_or(reference),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(text: &str, ruby: Option<&str>) -> Token {
        Token::Text {
            text: text.into(),
            ruby: ruby.map(Into::into),
        }
    }

    #[test]
    fn parses_ruby_and_entities() {
        let tokens = parse("[僕|ぼく]は{hero:13|ヘイズ}と{item:1}。");
        assert_eq!(
            tokens,
            vec![
                text("僕", Some("ぼく")),
                text("は", None),
                Token::Entity {
                    reference: "hero:13".into(),
                    spoken: Some("ヘイズ".into())
                },
                text("と", None),
                Token::Entity {
                    reference: "item:1".into(),
                    spoken: None
                },
                text("。", None),
            ]
        );
        assert_eq!(plain(&tokens), "僕はヘイズとitem:1。");
    }

    #[test]
    fn entities_only_keeps_brackets_and_unclosed_braces() {
        assert_eq!(
            parse_entities("[GG] {hero:13} wins {oops"),
            vec![
                text("[GG] ", None),
                Token::Entity {
                    reference: "hero:13".into(),
                    spoken: None
                },
                text(" wins {oops", None),
            ]
        );
    }
}
