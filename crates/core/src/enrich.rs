//! 確定した字幕に、固有名詞・ふりがな・翻訳を付ける（DESIGN.md 9 章）。
//!
//! - 固有名詞: 辞書の名前と略称を最長一致で探す（コードで決める）
//! - ふりがな: 形態素解析（`furigana`）で決める
//! - 翻訳と雑音の判定: LLM。固有名詞は `{id}` で書かせ、描画時に各言語の公式名にする
//!
//! LLM にトークン分けや読みまで任せると、区切りやルビがずれるので任せない。

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::furigana::Furigana;
use crate::lang;
use crate::markup;
use crate::types::{Enriched, EntityKind, Glossary, LangCode, Token};

const CHAT_URL: &str = "https://api.mistral.ai/v1/chat/completions";
const TIMEOUT: Duration = Duration::from_secs(8);

pub struct Enricher {
    http: reqwest::Client,
    api_key: String,
    model: String,
    targets: Vec<LangCode>,
    /// 固定の先頭部分（プロンプトキャッシュが効く）
    system: String,
    /// 参照 → 各言語の名前
    names: HashMap<String, BTreeMap<LangCode, String>>,
    matcher: Matcher,
    /// 日本語のときだけ
    furigana: Option<Furigana>,
}

impl Enricher {
    pub fn new(
        api_key: String,
        model: String,
        source_lang: LangCode,
        targets: Vec<LangCode>,
        glossary: Option<&Glossary>,
    ) -> anyhow::Result<Self> {
        let targets = lang::normalize(targets.into_iter().filter(|t| *t != source_lang));
        let furigana = (source_lang == "ja").then(Furigana::new).transpose()?;
        Ok(Self {
            http: reqwest::Client::new(),
            api_key,
            model,
            system: system_prompt(&source_lang, glossary),
            matcher: glossary
                .map(|g| Matcher::new(g, &source_lang))
                .unwrap_or_default(),
            names: glossary
                .map(|g| {
                    g.entities
                        .iter()
                        .map(|(r, e)| (r.clone(), e.names.clone()))
                        .collect()
                })
                .unwrap_or_default(),
            targets,
            furigana,
        })
    }

    /// `context` は直前の確定字幕（古い順）。翻訳に失敗しても、原文の固有名詞とふりがなは返す。
    /// 雑音（咳やフィラーだけ）はバッチの文字起こしが空になるので、ここでは判定しない
    pub async fn enrich(&self, text: &str, context: &[String]) -> Enriched {
        let spans = self.matcher.split(text);
        let source = self.source_tokens(&spans);
        let translations = match self.translate(&markup_of(&spans), context).await {
            Ok(raw) => self.translations(raw, &spans),
            Err(e) => {
                tracing::warn!("translate: {e:#}");
                BTreeMap::new()
            }
        };
        Enriched {
            source,
            translations,
        }
    }

    fn source_tokens(&self, spans: &[Span]) -> Vec<Token> {
        spans
            .iter()
            .flat_map(|span| match span {
                Span::Entity { reference, spoken } => vec![Token::Entity {
                    reference: reference.clone(),
                    spoken: Some(spoken.clone()),
                }],
                Span::Text(text) => match &self.furigana {
                    Some(f) => f.annotate(text),
                    None => vec![Token::Text {
                        text: text.clone(),
                        ruby: None,
                    }],
                },
            })
            .collect()
    }

    async fn translate(
        &self,
        transcript: &str,
        context: &[String],
    ) -> anyhow::Result<BTreeMap<LangCode, String>> {
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": self.system },
                { "role": "user", "content": json!({ "context": context, "transcript": transcript }).to_string() },
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": "translation", "strict": true, "schema": self.schema() },
            },
        });
        let res = self
            .http
            .post(CHAT_URL)
            .bearer_auth(&self.api_key)
            .timeout(TIMEOUT)
            .json(&body)
            .send()
            .await?;
        let status = res.status();
        if !status.is_success() {
            anyhow::bail!("{status}: {}", res.text().await.unwrap_or_default());
        }
        let res: Value = res.json().await?;
        let content =
            message_text(&res["choices"][0]["message"]["content"]).context("no message content")?;
        let out: Output = serde_json::from_str(&content).context("invalid translation JSON")?;
        Ok(out.translations)
    }

    fn schema(&self) -> Value {
        let translations: serde_json::Map<String, Value> = self
            .targets
            .iter()
            .map(|t| (t.clone(), json!({ "type": "string" })))
            .collect();
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["translations"],
            "properties": {
                "translations": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": self.targets,
                    "properties": translations,
                },
            },
        })
    }

    /// 頼んでいない言語は捨て、辞書にない `{ref}` は出さない（名前の分からない ID を表示しない）。
    /// LLM が `{id}` にせず名前をそのまま書いたときは、原文にあるエンティティの名前を探して戻す
    fn translations(
        &self,
        raw: BTreeMap<LangCode, String>,
        source: &[Span],
    ) -> BTreeMap<LangCode, Vec<Token>> {
        raw.into_iter()
            .filter(|(lang, _)| self.targets.contains(lang))
            .map(|(lang, text)| {
                let tokens: Vec<Token> = markup::parse_entities(&text)
                    .into_iter()
                    .filter(|t| match t {
                        Token::Entity { reference, .. } => self.names.contains_key(reference),
                        Token::Text { .. } => true,
                    })
                    .collect();
                let tokens = link_names(tokens, &self.name_candidates(source, &lang));
                (lang, tokens)
            })
            .collect()
    }

    /// 原文に出てきたエンティティの、その言語と英語での名前（長い順）
    fn name_candidates(&self, source: &[Span], lang: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = source
            .iter()
            .filter_map(|s| match s {
                Span::Entity { reference, .. } => Some(reference),
                Span::Text(_) => None,
            })
            .filter_map(|r| Some((r, self.names.get(r)?)))
            .flat_map(|(r, names)| {
                [names.get(lang), names.get("en")]
                    .into_iter()
                    .flatten()
                    .map(move |n| (n.clone(), r.clone()))
            })
            .collect();
        out.sort_by_key(|(n, _)| std::cmp::Reverse(n.chars().count()));
        out.dedup();
        out
    }
}

/// 文字列の中の名前（単語の境界で区切られたもの）をエンティティのトークンにする
fn link_names(tokens: Vec<Token>, candidates: &[(String, String)]) -> Vec<Token> {
    if candidates.is_empty() {
        return tokens;
    }
    let mut out = Vec::new();
    for token in tokens {
        let Token::Text { text, ruby: None } = token else {
            out.push(token);
            continue;
        };
        let mut rest = text.as_str();
        'scan: while !rest.is_empty() {
            for (i, _) in rest.char_indices() {
                let found = candidates.iter().find(|(name, _)| {
                    rest[i..].starts_with(name.as_str())
                        && !rest[..i]
                            .chars()
                            .next_back()
                            .is_some_and(char::is_alphanumeric)
                        && !rest[i + name.len()..]
                            .chars()
                            .next()
                            .is_some_and(char::is_alphanumeric)
                });
                if let Some((name, reference)) = found {
                    if i > 0 {
                        out.push(Token::Text {
                            text: rest[..i].into(),
                            ruby: None,
                        });
                    }
                    out.push(Token::Entity {
                        reference: reference.clone(),
                        spoken: None,
                    });
                    rest = &rest[i + name.len()..];
                    continue 'scan;
                }
            }
            out.push(Token::Text {
                text: rest.into(),
                ruby: None,
            });
            break;
        }
    }
    out
}

#[derive(Deserialize)]
struct Output {
    translations: BTreeMap<LangCode, String>,
}

#[derive(Debug, PartialEq)]
enum Span {
    Text(String),
    Entity { reference: String, spoken: String },
}

/// LLM に渡す原文。固有名詞は `{id|発話された表現}` にする
fn markup_of(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|s| match s {
            Span::Text(t) => t.clone(),
            Span::Entity { reference, spoken } => format!("{{{reference}|{spoken}}}"),
        })
        .collect()
}

/// 辞書の名前と略称を最長一致で探す
#[derive(Default)]
struct Matcher {
    /// 長い順
    terms: Vec<(Vec<char>, String)>,
}

impl Matcher {
    /// 1 文字の語は誤検出が多いので使わない
    const MIN_CHARS: usize = 2;

    fn new(glossary: &Glossary, lang: &str) -> Self {
        let mut terms: Vec<(Vec<char>, String)> = Vec::new();
        for (reference, entity) in &glossary.entities {
            let names = entity.names.get(lang).into_iter();
            let aliases = entity.aliases.get(lang).into_iter().flatten();
            for term in names.chain(aliases) {
                let chars: Vec<char> = term.chars().collect();
                // 同じ表記を複数のエンティティが持つときは、先に見つけたものにする
                if chars.len() >= Self::MIN_CHARS && !terms.iter().any(|(t, _)| *t == chars) {
                    terms.push((chars, reference.clone()));
                }
            }
        }
        terms.sort_by_key(|(t, _)| std::cmp::Reverse(t.len()));
        Self { terms }
    }

    fn split(&self, text: &str) -> Vec<Span> {
        let chars: Vec<char> = text.chars().collect();
        let mut spans = Vec::new();
        let mut buf = String::new();
        let mut i = 0;
        while i < chars.len() {
            let found = self
                .terms
                .iter()
                .find(|(term, _)| Self::matches_at(&chars, i, term));
            match found {
                Some((term, reference)) => {
                    if !buf.is_empty() {
                        spans.push(Span::Text(std::mem::take(&mut buf)));
                    }
                    spans.push(Span::Entity {
                        reference: reference.clone(),
                        spoken: term.iter().collect(),
                    });
                    i += term.len();
                }
                None => {
                    buf.push(chars[i]);
                    i += 1;
                }
            }
        }
        if !buf.is_empty() {
            spans.push(Span::Text(buf));
        }
        spans
    }

    /// カタカナの語は、前後もカタカナなら別の語の一部なので一致としない（「セブンイレブン」の「セブン」）
    fn matches_at(chars: &[char], i: usize, term: &[char]) -> bool {
        if !chars[i..].starts_with(term) {
            return false;
        }
        let katakana = |c: &char| matches!(c, '\u{30A0}'..='\u{30FF}');
        if !term.iter().all(katakana) {
            return true;
        }
        let before = i.checked_sub(1).and_then(|j| chars.get(j));
        let after = chars.get(i + term.len());
        !before.is_some_and(katakana) && !after.is_some_and(katakana)
    }
}

/// 推論するモデルは content が [{type: "thinking"}, {type: "text"}] の配列になる
fn message_text(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter(|p| p["type"] == "text")
                .filter_map(|p| p["text"].as_str())
                .collect(),
        ),
        _ => None,
    }
}

fn system_prompt(source_lang: &str, glossary: Option<&Glossary>) -> String {
    let lang_label = lang::find(source_lang).map_or(source_lang, |l| l.label);
    let mut prompt = format!(
        "You translate live commentary from a streamer playing the video game Deadlock. \
         The streamer speaks {lang_label}. Each request has \"transcript\" (one utterance from speech recognition) \
         and \"context\" (the previous utterances, for reference only; do not translate them).\n\
         In the transcript, game terms are marked as {{id|what the streamer said}}, for example {{hero:13|ヘイズ}}.\n\
         \n\
         Rules:\n\
         1. \"translations\": translate only the transcript into each requested language, naturally and casually, like gamers talk.\n\
         2. Write every marked game term only as {{id}}, for example {{hero:13}}. Never write the term's name, \
         and do not attach possessive endings to it (write \"the {{ability:1}} of {{hero:13}}\", not \"{{hero:13}}'s\").\n\
         3. Keep common gamer words natural (ult, farm, push, gank).\n"
    );
    let Some(glossary) = glossary else {
        return prompt;
    };
    prompt.push_str("\nGame terms (id: English name):\n");
    for kind in [EntityKind::Hero, EntityKind::Item, EntityKind::Ability] {
        for (id, e) in glossary.entities.iter().filter(|(_, e)| e.kind == kind) {
            if let Some(name) = e.names.get("en") {
                prompt.push_str(&format!("{id}: {name}\n"));
            }
        }
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glossary() -> Glossary {
        serde_json::from_value(json!({
            "clientVersion": 1, "languages": ["ja", "en"],
            "entities": {
                "hero:13": { "kind": "hero", "className": "h", "names": { "ja": "ヘイズ", "en": "Haze" }, "aliases": {} },
                "hero:2": { "kind": "hero", "className": "s", "names": { "ja": "セブン", "en": "Seven" }, "aliases": {} },
                "item:1": { "kind": "item", "className": "i", "names": { "ja": "基礎拡張マガジン", "en": "Extended Magazine" }, "aliases": { "ja": ["拡張マガジン"] } },
                "ability:9": { "kind": "ability", "className": "a", "names": { "ja": "ダンス", "en": "Dance" }, "aliases": {} },
                "ability:8": { "kind": "ability", "className": "b", "names": { "ja": "バレットダンス", "en": "Bullet Dance" }, "aliases": {} }
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

    #[test]
    fn matcher_prefers_longest_and_aliases() {
        let m = Matcher::new(&glossary(), "ja");
        assert_eq!(
            m.split("ヘイズのバレットダンスと拡張マガジン"),
            [
                entity("hero:13", "ヘイズ"),
                Span::Text("の".into()),
                entity("ability:8", "バレットダンス"),
                Span::Text("と".into()),
                entity("item:1", "拡張マガジン"),
            ]
        );
    }

    #[test]
    fn matcher_skips_katakana_inside_other_words() {
        let m = Matcher::new(&glossary(), "ja");
        assert_eq!(
            m.split("セブンイレブンでセブン"),
            [
                Span::Text("セブンイレブンで".into()),
                entity("hero:2", "セブン")
            ]
        );
    }

    #[test]
    fn markup_marks_entities_for_the_llm() {
        let m = Matcher::new(&glossary(), "ja");
        assert_eq!(markup_of(&m.split("ヘイズ強い")), "{hero:13|ヘイズ}強い");
    }

    fn enricher() -> Enricher {
        Enricher::new(
            "key".into(),
            "model".into(),
            "ja".into(),
            vec!["en".into(), "ja".into(), "xx".into()],
            Some(&glossary()),
        )
        .unwrap()
    }

    #[test]
    fn source_tokens_combine_entities_and_furigana() {
        let e = enricher();
        let tokens = e.source_tokens(&e.matcher.split("ヘイズは強い"));
        assert_eq!(
            tokens,
            [
                Token::Entity {
                    reference: "hero:13".into(),
                    spoken: Some("ヘイズ".into())
                },
                Token::Text {
                    text: "は".into(),
                    ruby: None
                },
                Token::Text {
                    text: "強".into(),
                    ruby: Some("つよ".into())
                },
                Token::Text {
                    text: "い".into(),
                    ruby: None
                },
            ]
        );
    }

    #[test]
    fn targets_exclude_source_and_unknown_languages() {
        assert_eq!(enricher().targets, ["en"]);
    }

    #[test]
    fn translations_drop_unknown_refs_and_languages() {
        let raw = BTreeMap::from([
            (
                "en".to_owned(),
                "{hero:13} is strong, {hero:999} too".to_owned(),
            ),
            ("ru".to_owned(), "ignored".to_owned()),
        ]);
        let t = enricher().translations(raw, &[]);
        assert_eq!(t.len(), 1);
        assert_eq!(markup::plain(&t["en"]), "hero:13 is strong,  too");
    }

    #[test]
    fn plain_names_in_translations_become_entities() {
        let e = enricher();
        let source = e.matcher.split("ヘイズのバレットダンス");
        let raw = BTreeMap::from([(
            "en".to_owned(),
            "Haze's {ability:8} beats Hazel and Seven".to_owned(),
        )]);
        let t = &e.translations(raw, &source)["en"];
        assert_eq!(
            t,
            &[
                Token::Entity {
                    reference: "hero:13".into(),
                    spoken: None
                },
                Token::Text {
                    text: "'s ".into(),
                    ruby: None
                },
                Token::Entity {
                    reference: "ability:8".into(),
                    spoken: None
                },
                // Hazel は単語の途中、Seven は原文に出てこないので名前にしない
                Token::Text {
                    text: " beats Hazel and Seven".into(),
                    ruby: None
                },
            ]
        );
    }

    #[test]
    fn message_text_handles_thinking_parts() {
        let content = json!([{ "type": "thinking", "thinking": [] }, { "type": "text", "text": "{\"a\":1}" }]);
        assert_eq!(message_text(&content).as_deref(), Some("{\"a\":1}"));
    }

    #[test]
    fn schema_requires_each_target_language() {
        let schema = enricher().schema();
        assert_eq!(
            schema["properties"]["translations"]["required"],
            json!(["en"])
        );
    }
}
