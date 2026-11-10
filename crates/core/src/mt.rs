//! 翻訳専門のモデル（Qwen-MT-Flash）で訳す（DESIGN.md 9.7）。
//!
//! 用語は先に決めてあるので、英語名に置き換えて `⟦Haze⟧` と囲んだ文を渡し、訳文に残った名前を用語に戻す。
//! 翻訳先ごとに 1 回ずつ、同時に聞く（1 回の応答は 0.2〜0.3 秒）。括弧を外したり引用符に替えたりすることが
//! あるので、括弧がなくても英語名があれば用語にする。名前まで訳された用語は、ふつうの文字のまま出る

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::future::join_all;
use serde_json::{Value, json};

use crate::access::Access;
use crate::enrich::Span;
use crate::lang;
use crate::types::{Glossary, LangCode, Token, Translations};
use crate::usage::{Meter, Tokens, Use};

const MODEL: &str = "qwen-mt-flash";

const TIMEOUT: Duration = Duration::from_secs(5);

/// 回数の上限（429）で断られたら、1 度だけ待って送り直す
const RETRY_AFTER: Duration = Duration::from_millis(700);

/// 訳し方の指示。これがないと、括弧を外すことが多い（実測）
const DOMAINS: &str = "Casual live commentary by a streamer playing the video game Deadlock, like gamers talk. \
     Words in ⟦ ⟧ are game terms (names of heroes, items and abilities); \
     keep them exactly as they are, with the brackets.";

pub struct Translator {
    access: Access,
    source_lang: LangCode,
    targets: Vec<LangCode>,
    meter: Arc<Meter>,
}

impl Translator {
    pub fn new(
        access: Access,
        source_lang: LangCode,
        targets: Vec<LangCode>,
        meter: Arc<Meter>,
    ) -> Self {
        Self {
            access,
            source_lang,
            targets,
            meter,
        }
    }

    /// 用語を決めた文を、翻訳先ごとに訳す。訳せなかった言語は含めない
    pub async fn translate(
        &self,
        http: &reqwest::Client,
        spans: &[Span],
        glossary: Option<&Glossary>,
    ) -> Translations {
        let (source, terms) = tagged(spans, glossary);
        let (source, terms) = (&source, &terms);
        let asked = self.targets.iter().map(|target| async move {
            let translated = self.ask(http, source, target).await;
            match translated {
                Ok(text) => Some((target.clone(), tokens(&text, terms))),
                Err(e) => {
                    tracing::warn!("translation ({target}): {e:#}");
                    None
                }
            }
        });
        join_all(asked)
            .await
            .into_iter()
            .flatten()
            .filter(|(_, tokens)| !tokens.is_empty())
            .collect()
    }

    async fn ask(
        &self,
        http: &reqwest::Client,
        source: &str,
        target: &str,
    ) -> anyhow::Result<String> {
        let body = json!({
            "model": MODEL,
            "messages": [{ "role": "user", "content": source }],
            "translation_options": {
                "source_lang": code(&self.source_lang),
                "target_lang": code(target),
                "domains": DOMAINS,
            },
        });
        let send = || {
            http.post(self.access.url("/chat/completions"))
                .bearer_auth(self.access.token())
                .timeout(TIMEOUT)
                .json(&body)
                .send()
        };
        let mut res = send().await?;
        if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            tokio::time::sleep(RETRY_AFTER).await;
            res = send().await?;
        }
        let status = res.status();
        let res: Value = res.json().await?;
        anyhow::ensure!(status.is_success(), "{status}: {res}");
        let usage = &res["usage"];
        let n = |key: &str| usage[key].as_u64().unwrap_or(0);
        self.meter.tokens(
            Use::Translate,
            MODEL,
            Tokens {
                input: n("prompt_tokens"),
                cached: 0,
                output: n("completion_tokens"),
            },
        );
        res["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_owned)
            .context("no translation")
    }
}

/// Qwen-MT の言語コード。地域は分けない（中国語の簡体と繁体だけ別）
fn code(lang: &str) -> &str {
    match lang {
        "zh-Hans" => "zh",
        "zh-Hant" => "zh_tw",
        "no" => "nb",
        _ => lang::find(lang).map_or(lang, |l| l.stt),
    }
}

/// 用語を `⟦英語名⟧` に置き換えた文と、その文の用語（英語名と参照）。英語名のない用語は話した言葉のまま
fn tagged(spans: &[Span], glossary: Option<&Glossary>) -> (String, Vec<(String, String)>) {
    let mut source = String::new();
    let mut terms: Vec<(String, String)> = Vec::new();
    for span in spans {
        let named = match span {
            Span::Entity { reference, .. } => glossary
                .and_then(|g| g.entities.get(reference)?.names.get("en"))
                .map(|name| (name, reference)),
            Span::Text(_) => None,
        };
        match (span, named) {
            (_, Some((name, reference))) => {
                source += &format!("⟦{name}⟧");
                if !terms.iter().any(|(n, _)| n == name) {
                    terms.push((name.clone(), reference.clone()));
                }
            }
            (Span::Text(text), None) => source += text,
            (Span::Entity { spoken, .. }, None) => source += spoken,
        }
    }
    // 長い名前を先に探す（"Extra Spirit" の中の "Spirit" を別の用語にしない）
    terms.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    (source, terms)
}

/// 訳文の中の英語名を用語にする。名前を囲む括弧と引用符は外す
fn tokens(translation: &str, terms: &[(String, String)]) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut text = String::new();
    let mut rest = translation;
    while !rest.is_empty() {
        let term = terms
            .iter()
            .find(|(name, _)| rest.starts_with(name.as_str()));
        let Some((name, reference)) = term else {
            let ch = rest.chars().next().expect("not empty");
            text.push(ch);
            rest = &rest[ch.len_utf8()..];
            continue;
        };
        rest = &rest[name.len()..];
        // 名前を囲んでいた括弧や引用符（⟦Haze⟧、«Haze»、"Haze"）を外す
        let closing = text
            .chars()
            .next_back()
            .and_then(|open| QUOTES.iter().find(|(o, _)| *o == open))
            .filter(|(_, close)| rest.starts_with(*close));
        if let Some((open, close)) = closing {
            text.truncate(text.len() - open.len_utf8());
            rest = &rest[close.len_utf8()..];
        }
        push_text(&mut tokens, &mut text);
        tokens.push(Token::Entity {
            reference: reference.clone(),
            spoken: None,
            ruby: None,
        });
    }
    push_text(&mut tokens, &mut text);
    tokens
}

const QUOTES: [(char, char); 6] = [
    ('⟦', '⟧'),
    ('«', '»'),
    ('"', '"'),
    ('“', '”'),
    ('„', '“'),
    ('「', '」'),
];

/// 名前まで訳された用語の括弧（中は訳された名前）は、括弧だけ外して文字として出す
fn push_text(tokens: &mut Vec<Token>, text: &mut String) {
    let plain = std::mem::take(text).replace(['⟦', '⟧'], "");
    if !plain.is_empty() {
        tokens.push(Token::Text {
            text: plain,
            ruby: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glossary() -> Glossary {
        serde_json::from_value(json!({
            "clientVersion": 1, "languages": ["ja", "en"],
            "entities": {
                "hero:13": { "kind": "hero", "className": "h", "names": { "ja": "ヘイズ", "en": "Haze" } },
                "ability:4": { "kind": "ability", "className": "b", "names": { "ja": "バレットダンス", "en": "Bullet Dance" } },
                "item:9": { "kind": "item", "className": "i", "names": { "ja": "名前だけ" } }
            }
        }))
        .unwrap()
    }

    fn entity(reference: &str, spoken: &str) -> Span {
        Span::Entity {
            reference: reference.into(),
            spoken: spoken.into(),
        }
    }

    fn text(t: &str) -> Token {
        Token::Text {
            text: t.into(),
            ruby: None,
        }
    }

    fn term(reference: &str) -> Token {
        Token::Entity {
            reference: reference.into(),
            spoken: None,
            ruby: None,
        }
    }

    fn terms() -> Vec<(String, String)> {
        vec![
            ("Bullet Dance".into(), "ability:4".into()),
            ("Haze".into(), "hero:13".into()),
        ]
    }

    #[test]
    fn terms_are_sent_as_english_names_in_brackets() {
        let spans = [
            entity("hero:13", "ヘイズ"),
            Span::Text("の".into()),
            entity("ability:4", "ウルト"),
            Span::Text("と".into()),
            entity("item:9", "名前だけ"),
            Span::Text("は強い。".into()),
        ];
        let (source, found) = tagged(&spans, Some(&glossary()));
        assert_eq!(source, "⟦Haze⟧の⟦Bullet Dance⟧と名前だけは強い。");
        assert_eq!(found, terms());
        // 辞書がなければ、話した言葉のまま訳す
        assert_eq!(tagged(&spans, None).0, "ヘイズのウルトと名前だけは強い。");
    }

    #[test]
    fn names_left_in_the_translation_become_terms() {
        assert_eq!(
            tokens("⟦Haze⟧'s ⟦Bullet Dance⟧ is strong.", &terms()),
            [
                term("hero:13"),
                text("'s "),
                term("ability:4"),
                text(" is strong.")
            ]
        );
    }

    #[test]
    fn names_are_found_without_brackets_or_in_quotes() {
        assert_eq!(
            tokens("«Bullet Dance» от Haze очень сильна.", &terms()),
            [
                term("ability:4"),
                text(" от "),
                term("hero:13"),
                text(" очень сильна.")
            ]
        );
    }

    #[test]
    fn a_translated_name_stays_as_text_without_brackets() {
        assert_eq!(
            tokens("⟦Хейз⟧ с ⟦Bullet Dance⟧.", &terms()),
            [text("Хейз с "), term("ability:4"), text(".")]
        );
    }

    #[test]
    fn language_codes_follow_the_model() {
        assert_eq!(code("ja"), "ja");
        assert_eq!(code("zh-Hans"), "zh");
        assert_eq!(code("zh-Hant"), "zh_tw");
        assert_eq!(code("pt-BR"), "pt");
        assert_eq!(code("no"), "nb");
    }
}
