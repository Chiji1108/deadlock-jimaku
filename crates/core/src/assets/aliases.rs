//! 用語の別名（略称や、英語名のカタカナ。DESIGN.md 9.7）。
//!
//! 用語を Decisions API で確かめるとき、文の中の候補を探すのに使う。人の手では書かず、辞書の読みと同じく
//! LLM に一度だけ作らせて、名前ごとに置いておく（パッチで用語が増えたら、次の起動で足りない分だけ聞く）。
//! 別名は候補を出すだけで、本当にその用語かは文ごとに確かめるので、少し多めに出てもよい

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use super::readings::ReadingModel;
use crate::llm;
use crate::types::{EntityKind, Glossary};
use crate::usage::Use;

/// 60 語で 20 秒ほど
const TIMEOUT: Duration = Duration::from_secs(120);

/// 1 つの用語の別名の上限
const MAX_ALIASES: usize = 5;

/// 「日本語の名前|英語名」 → 別名。名前ごとなので、パッチやキャッシュ形式をまたいで使い回す
pub type Cache = BTreeMap<String, Vec<String>>;

/// 参照 → 別名
pub type Aliases = BTreeMap<String, Vec<String>>;

fn key(ja: &str, en: &str) -> String {
    format!("{ja}|{en}")
}

/// 日本語と英語の名前がある用語（キャッシュのキー、問い合わせの行）
fn named(glossary: &Glossary) -> impl Iterator<Item = (&String, String, String)> {
    glossary.entities.iter().filter_map(|(reference, e)| {
        let (ja, en) = (e.names.get("ja")?, e.names.get("en")?);
        let kind = match e.kind {
            EntityKind::Hero => "hero",
            EntityKind::Item => "item",
            EntityKind::Ability => "ability",
        };
        Some((reference, key(ja, en), format!("{kind} | {ja} | {en}")))
    })
}

/// キャッシュにある別名を、参照ごとに並べる
pub fn by_reference(glossary: &Glossary, cache: &Cache) -> Aliases {
    named(glossary)
        .filter_map(|(reference, key, _)| Some((reference.clone(), cache.get(&key)?.clone())))
        .collect()
}

/// まだ問い合わせていない用語（キーと、問い合わせの行）
pub fn missing(glossary: &Glossary, cache: &Cache) -> Vec<(String, String)> {
    let terms: BTreeMap<String, String> = named(glossary)
        .filter(|(_, key, _)| !cache.contains_key(key))
        .map(|(_, key, line)| (key, line))
        .collect();
    terms.into_iter().collect()
}

/// `terms`（`missing` の一部）の別名を問い合わせる。答えのない用語は含めない（次の起動で問い直す）
pub async fn ask(
    http: &reqwest::Client,
    model: &ReadingModel,
    terms: &[(String, String)],
) -> anyhow::Result<Cache> {
    #[derive(Deserialize)]
    struct Output {
        terms: Vec<Term>,
    }
    #[derive(Deserialize)]
    struct Term {
        id: usize,
        aliases: Vec<String>,
    }

    let schema = json!({
        "type": "object", "additionalProperties": false, "required": ["terms"],
        "properties": { "terms": { "type": "array", "items": {
            "type": "object", "additionalProperties": false, "required": ["id", "aliases"],
            "properties": { "id": { "type": "integer" }, "aliases": { "type": "array", "items": { "type": "string" } } },
        }}},
    });
    let lines: Vec<String> = terms
        .iter()
        .enumerate()
        .map(|(id, (_, line))| format!("{id} | {line}"))
        .collect();
    let out: Output = llm::chat_json(
        http,
        llm::Request {
            access: &model.access,
            model: &model.model,
            reasoning_effort: model.reasoning_effort.as_deref(),
            system: SYSTEM,
            user: lines.join("\n"),
            schema_name: "aliases",
            schema,
            timeout: TIMEOUT,
            meter: &model.meter,
            purpose: Use::Readings,
        },
    )
    .await?;
    Ok(out
        .terms
        .into_iter()
        .filter_map(|term| {
            let (key, line) = terms.get(term.id)?;
            Some((key.clone(), cleaned(term.aliases, line)))
        })
        .collect())
}

/// 短すぎるもの、名前そのもの、重なりを除く
fn cleaned(aliases: Vec<String>, line: &str) -> Vec<String> {
    let names: Vec<&str> = line.split(" | ").skip(1).collect();
    let mut kept: Vec<String> = Vec::new();
    for alias in aliases {
        let alias = alias.trim();
        if alias.chars().count() >= 2 && !names.contains(&alias) && !kept.iter().any(|k| k == alias)
        {
            kept.push(alias.to_owned());
        }
    }
    kept.truncate(MAX_ALIASES);
    kept
}

const SYSTEM: &str = "You know how Japanese streamers talk about the video game Deadlock. For each term \
(id | kind | Japanese name | English name), list the other ways a Japanese streamer may say it aloud, \
written as speech recognition would write them: abbreviations and nicknames of the Japanese name \
(\"拡マガ\" for \"基礎拡張マガジン\"), and the English name or its short form in katakana. \
Give 0 to 5 per term, most likely first. Do not list the Japanese name itself, ordinary everyday words, \
or generic words that fit many terms (\"ウルト\", numbers).";

#[cfg(test)]
mod tests {
    use super::*;

    fn glossary() -> Glossary {
        serde_json::from_value(json!({
            "clientVersion": 1, "languages": ["ja", "en"],
            "entities": {
                "item:1": { "kind": "item", "className": "i", "names": { "ja": "基礎拡張マガジン", "en": "Extended Magazine" } },
                "hero:1": { "kind": "hero", "className": "h", "names": { "ja": "ヘイズ", "en": "Haze" } },
                "hero:2": { "kind": "hero", "className": "g", "names": { "en": "Unnamed" } }
            }
        }))
        .unwrap()
    }

    #[test]
    fn cached_aliases_are_keyed_by_name_and_listed_by_reference() {
        let g = glossary();
        let cache = Cache::from([(
            key("基礎拡張マガジン", "Extended Magazine"),
            vec!["拡マガ".to_owned()],
        )]);
        assert_eq!(
            by_reference(&g, &cache),
            Aliases::from([("item:1".to_owned(), vec!["拡マガ".to_owned()])])
        );
        // 日本語の名前のない用語は聞かない
        assert_eq!(
            missing(&g, &cache),
            [(key("ヘイズ", "Haze"), "hero | ヘイズ | Haze".to_owned())]
        );
    }

    #[test]
    fn answers_are_cleaned() {
        let aliases =
            ["拡マガ", "基礎拡張マガジン", "拡", " 拡マガ ", "エクマガ"].map(String::from);
        assert_eq!(
            cleaned(
                aliases.to_vec(),
                "item | 基礎拡張マガジン | Extended Magazine"
            ),
            ["拡マガ", "エクマガ"]
        );
    }
}
