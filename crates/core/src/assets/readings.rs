//! 辞書の漢字名の読み（DESIGN.md 8.2）。
//!
//! 辞書には読みを保存せず、読み込むたびに LLM の読み（名前ごとのキャッシュ）を重ねる。なければ読みなしで、
//! 用語の判定は Lindera + IPADIC が名前を読んだ結果を使う。
//! キャッシュにない名前は辞書を配信した後に問い合わせ、届いたら配信し直す。
//! 答えが検証に通らなかった名前はキャッシュせず、次の起動で問い直す（答えは揺れるので、たいてい次は通る）。
//! 辞書の保存と切り離しているので、キーが無効でも辞書は保存され、キーを後から設定しても次の起動で効く。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::access::Access;
use crate::furigana;
use crate::llm;
use crate::types::{Glossary, RubySegment};
use crate::usage::{Meter, Use};

/// 35 語で 20 秒ほど
const TIMEOUT: Duration = Duration::from_secs(120);

pub struct ReadingModel {
    pub access: Access,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub meter: Arc<Meter>,
}

/// 名前 → 読み
pub type Cache = BTreeMap<String, Vec<RubySegment>>;

/// ファイルがなければ空。壊れていたら脇に退けて空から始める。
/// 読めない（権限など）、退けられないときは Err（黙って上書きしない）
pub async fn load_cache(path: &Path) -> anyhow::Result<Cache> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Cache::new()),
        Err(e) => return Err(e.into()),
    };
    match serde_json::from_slice(&bytes) {
        Ok(cache) => Ok(cache),
        Err(e) => {
            let aside = path.with_extension("json.broken");
            tokio::fs::rename(path, &aside).await?;
            tracing::warn!(
                "readings cache is broken ({e}); moved to {}",
                aside.display()
            );
            Ok(Cache::new())
        }
    }
}

pub async fn save_cache(path: &Path, cache: &Cache) -> anyhow::Result<()> {
    crate::write_atomic(path, &serde_json::to_vec_pretty(cache)?).await
}

/// LLM に読ませる名前（漢字を含むもの）
fn kanji_names(glossary: &Glossary) -> impl Iterator<Item = (&String, &String)> {
    glossary
        .entities
        .iter()
        .filter_map(|(r, e)| Some((r, e.names.get("ja")?)))
        .filter(|(_, name)| name.chars().any(furigana::is_kanji))
}

/// キャッシュにある LLM の読みを辞書に重ねる
pub fn apply(glossary: &mut Glossary, cache: &Cache) {
    let found: Vec<(String, Vec<RubySegment>)> = kanji_names(glossary)
        .filter_map(|(r, name)| Some((r.clone(), cache.get(name)?.clone())))
        .collect();
    for (reference, ruby) in found {
        if let Some(e) = glossary.entities.get_mut(&reference) {
            e.ruby = Some(ruby);
        }
    }
}

/// まだ問い合わせていない名前
pub fn missing(glossary: &Glossary, cache: &Cache) -> Vec<String> {
    let names: BTreeSet<&String> = kanji_names(glossary)
        .map(|(_, name)| name)
        .filter(|name| !cache.contains_key(*name))
        .collect();
    names.into_iter().cloned().collect()
}

/// `names` の読みを問い合わせる。答えがない・検証に通らない名前は含めない
pub async fn ask(
    http: &reqwest::Client,
    model: &ReadingModel,
    names: &[String],
) -> anyhow::Result<Cache> {
    #[derive(Deserialize)]
    struct Output {
        names: Vec<Named>,
    }
    #[derive(Deserialize)]
    struct Named {
        name: String,
        segments: Vec<RubySegment>,
    }

    let segment = json!({
        "type": "object", "additionalProperties": false, "required": ["text", "ruby"],
        "properties": { "text": { "type": "string" }, "ruby": { "type": ["string", "null"] } },
    });
    let schema = json!({
        "type": "object", "additionalProperties": false, "required": ["names"],
        "properties": { "names": { "type": "array", "items": {
            "type": "object", "additionalProperties": false, "required": ["name", "segments"],
            "properties": { "name": { "type": "string" }, "segments": { "type": "array", "items": segment } },
        }}},
    });
    let out: Output = llm::chat_json(
        http,
        llm::Request {
            access: &model.access,
            model: &model.model,
            reasoning_effort: model.reasoning_effort.as_deref(),
            system: SYSTEM,
            user: serde_json::to_string(names)?,
            schema_name: "readings",
            schema,
            timeout: TIMEOUT,
            meter: &model.meter,
            purpose: Use::Readings,
        },
    )
    .await?;
    let mut answers: BTreeMap<String, Vec<RubySegment>> = out
        .names
        .into_iter()
        .map(|n| (n.name, n.segments))
        .collect();
    Ok(names
        .iter()
        .filter_map(|name| {
            let Some(segments) = answers.remove(name) else {
                tracing::warn!("readings: no answer for {name}");
                return None;
            };
            let answer = serde_json::to_string(&segments).unwrap_or_default();
            let checked = furigana::check(name, segments);
            if checked.is_none() {
                tracing::warn!("readings: rejected {name}: {answer}");
            }
            Some((name.clone(), checked?))
        })
        .collect())
}

const SYSTEM: &str = "These are Japanese names of heroes' abilities and items in the video game Deadlock (Japanese localization). \
For each name, split it into words and give the hiragana reading of each word that contains kanji. \
A kanji compound is one segment with the reading of the whole word (退避 → たいひ, 人狼 → じんろう); never split a compound into single kanji. \
Keep okurigana and kana in their own segments with ruby null. Joining the segments' text must give the name exactly. \
Many names are coined compounds; read them the way a Japanese gamer would read them aloud.";

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str, ruby: Option<&str>) -> RubySegment {
        RubySegment {
            text: text.into(),
            ruby: ruby.map(Into::into),
        }
    }

    fn glossary() -> Glossary {
        serde_json::from_value(json!({
            "clientVersion": 1, "languages": ["ja"],
            "entities": {
                "ability:1": { "kind": "ability", "className": "a", "names": { "ja": "人狼の呪い" },
                               "ruby": [{ "text": "人", "ruby": "ひと" }, { "text": "狼", "ruby": "おおかみ" }, { "text": "の呪い" }] },
                "item:1": { "kind": "item", "className": "i", "names": { "ja": "基礎拡張マガジン" } },
                "item:2": { "kind": "item", "className": "j", "names": { "ja": "穴掘り" } },
                "hero:1": { "kind": "hero", "className": "h", "names": { "ja": "ヘイズ" } }
            }
        }))
        .unwrap()
    }

    #[test]
    fn apply_overlays_cached_readings() {
        let mut g = glossary();
        let cache = Cache::from([(
            "人狼の呪い".to_owned(),
            vec![seg("人狼", Some("じんろう")), seg("の呪い", None)],
        )]);
        apply(&mut g, &cache);
        assert_eq!(
            g.entities["ability:1"].ruby.as_ref().unwrap()[0]
                .ruby
                .as_deref(),
            Some("じんろう")
        );
        assert_eq!(g.entities["item:2"].ruby, None);
    }

    #[test]
    fn missing_skips_cached_and_kana_names() {
        let g = glossary();
        let cache = Cache::from([("穴掘り".to_owned(), vec![seg("穴", Some("あな"))])]);
        assert_eq!(missing(&g, &cache), ["人狼の呪い", "基礎拡張マガジン"]);
    }

    #[tokio::test]
    async fn broken_cache_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        tokio::fs::write(&path, b"{ broken").await.unwrap();
        assert!(load_cache(&path).await.unwrap().is_empty());
        assert!(dir.path().join("cache.json.broken").exists());
    }
}
