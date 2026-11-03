//! 短い記法から Token の列を作る。
//!
//! - `{hero:13}` / `{hero:13|ヘイズ}`: エンティティ（`|` の後は発話された表現）
//! - `[漢字|かんじ]`: ルビ
//!
//! 閉じていない括弧はそのまま文字として扱う。

use crate::types::Token;

/// ダミー字幕の記法を読む
pub fn parse(markup: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut rest = markup;
    let mut text = String::new();
    while let Some(start) = rest.find(['[', '{']) {
        let ruby = rest[start..].starts_with('[');
        let close = if ruby { ']' } else { '}' };
        // 閉じる前に次の括弧が開いたら、この括弧はただの文字
        let inner_end = rest[start + 1..]
            .find(['[', '{', close])
            .map(|i| start + 1 + i);
        let Some(end) = inner_end.filter(|&e| rest[e..].starts_with(close)) else {
            text.push_str(&rest[..=start]);
            rest = &rest[start + 1..];
            continue;
        };
        text.push_str(&rest[..start]);
        push_text(&mut tokens, &std::mem::take(&mut text), None);
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
                ruby: None,
            });
        }
        rest = &rest[end + 1..];
    }
    text.push_str(rest);
    push_text(&mut tokens, &text, None);
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
            Token::Entity {
                spoken, reference, ..
            } => spoken.as_deref().unwrap_or(reference),
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
                    spoken: Some("ヘイズ".into()),
                    ruby: None,
                },
                text("と", None),
                Token::Entity {
                    reference: "item:1".into(),
                    spoken: None,
                    ruby: None,
                },
                text("。", None),
            ]
        );
        assert_eq!(plain(&tokens), "僕はヘイズとitem:1。");
    }

    #[test]
    fn unclosed_brackets_stay_as_text() {
        assert_eq!(
            parse("lol {wow {hero:13} wins {oops"),
            vec![
                text("lol {wow ", None),
                Token::Entity {
                    reference: "hero:13".into(),
                    spoken: None,
                    ruby: None,
                },
                text(" wins {oops", None),
            ]
        );
    }
}
