//! 辞書の漢字名の読み（DESIGN.md 8.2）。
//!
//! 辞書には読みを保存せず、読み込むたびに重ねる。優先順は
//! 手で書いた読み（`data/readings.ja.json`）→ LLM の読み（名前ごとのキャッシュ）。どちらもなければ読みなし。
//! キャッシュにない名前は辞書を配信した後に問い合わせ、届いたら配信し直す。
//! 答えが検証に通らなかった名前はキャッシュせず、次の起動で問い直す（答えは揺れるので、たいてい次は通る）。
//! 辞書の保存と切り離しているので、キーが無効でも辞書は保存され、キーを後から設定しても次の起動で効く。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::furigana;
use crate::llm;
use crate::types::{Glossary, RubySegment};

/// 35 語で 20 秒ほど
const TIMEOUT: Duration = Duration::from_secs(120);

pub struct ReadingModel {
    pub api_key: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
}

/// 名前 → 読み
pub type Cache = BTreeMap<String, Vec<RubySegment>>;

/// 手で書いた読み。参照 → 読み（つなげると今の名前になるものだけが使われる）
pub type Manual = BTreeMap<String, Vec<RubySegment>>;

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
    tokio::fs::create_dir_all(path.parent().expect("has parent")).await?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, serde_json::to_vec_pretty(cache)?).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

/// その名前に使える手書きの読み（パッチで名前が変わっていたら使わない）
fn manual_for<'a>(manual: &'a Manual, reference: &str, name: &str) -> Option<&'a Vec<RubySegment>> {
    let ruby = manual.get(reference)?;
    let text: String = ruby.iter().map(|s| s.text.as_str()).collect();
    if text != name {
        tracing::warn!("readings.ja.json: {reference} does not match the name {name}");
        return None;
    }
    Some(ruby)
}

/// LLM に読ませる名前（漢字を含み、使える手書きの読みがないもの）
fn kanji_names<'a>(
    glossary: &'a Glossary,
    manual: &'a Manual,
) -> impl Iterator<Item = (&'a String, &'a String)> {
    glossary
        .entities
        .iter()
        .filter_map(|(r, e)| Some((r, e.names.get("ja")?)))
        .filter(|(_, name)| name.chars().any(furigana::is_kanji))
        .filter(|(r, name)| manual_for(manual, r, name).is_none())
}

/// 手書きの読みと、キャッシュにある LLM の読みを辞書に重ねる
pub fn apply(glossary: &mut Glossary, cache: &Cache, manual: &Manual) {
    let mut found: Vec<(String, Vec<RubySegment>)> = kanji_names(glossary, manual)
        .filter_map(|(r, name)| Some((r.clone(), cache.get(name)?.clone())))
        .collect();
    found.extend(glossary.entities.iter().filter_map(|(r, e)| {
        Some((
            r.clone(),
            manual_for(manual, r, e.names.get("ja")?)?.clone(),
        ))
    }));
    for (reference, ruby) in found {
        if let Some(e) = glossary.entities.get_mut(&reference) {
            e.ruby = Some(ruby);
        }
    }
}

/// まだ問い合わせていない名前
pub fn missing(glossary: &Glossary, cache: &Cache, manual: &Manual) -> Vec<String> {
    let names: BTreeSet<&String> = kanji_names(glossary, manual)
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
            api_key: &model.api_key,
            model: &model.model,
            reasoning_effort: model.reasoning_effort.as_deref(),
            system: SYSTEM,
            user: serde_json::to_string(names)?,
            schema_name: "readings",
            schema,
            timeout: TIMEOUT,
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
                "ability:1": { "kind": "ability", "className": "a", "names": { "ja": "人狼の呪い" }, "aliases": {},
                               "ruby": [{ "text": "人", "ruby": "ひと" }, { "text": "狼", "ruby": "おおかみ" }, { "text": "の呪い" }] },
                "item:1": { "kind": "item", "className": "i", "names": { "ja": "基礎拡張マガジン" }, "aliases": {} },
                "item:2": { "kind": "item", "className": "j", "names": { "ja": "穴掘り" }, "aliases": {} },
                "hero:1": { "kind": "hero", "className": "h", "names": { "ja": "ヘイズ" }, "aliases": {} }
            }
        }))
        .unwrap()
    }

    #[test]
    fn apply_overlays_cached_readings_except_manual_ones() {
        let mut g = glossary();
        let manual = Manual::from([
            (
                "item:1".to_owned(),
                vec![seg("基礎拡張", Some("てがき")), seg("マガジン", None)],
            ),
            // 名前が変わった手書きの読みは使わず、LLM の読みを使う
            (
                "item:2".to_owned(),
                vec![seg("古い名前", Some("ふるいなまえ"))],
            ),
        ]);
        let cache = Cache::from([
            (
                "人狼の呪い".to_owned(),
                vec![seg("人狼", Some("じんろう")), seg("の呪い", None)],
            ),
            ("基礎拡張マガジン".to_owned(), vec![seg("x", Some("x"))]),
            (
                "穴掘り".to_owned(),
                vec![
                    seg("穴", Some("あな")),
                    seg("掘", Some("ほ")),
                    seg("り", None),
                ],
            ),
        ]);
        apply(&mut g, &cache, &manual);
        assert_eq!(
            g.entities["ability:1"].ruby.as_ref().unwrap()[0]
                .ruby
                .as_deref(),
            Some("じんろう")
        );
        assert_eq!(
            g.entities["item:1"].ruby.as_ref().unwrap()[0]
                .ruby
                .as_deref(),
            Some("てがき")
        );
        assert_eq!(
            g.entities["item:2"].ruby.as_ref().unwrap()[0]
                .ruby
                .as_deref(),
            Some("あな")
        );
    }

    #[test]
    fn missing_skips_cached_manual_and_kana_names() {
        let g = glossary();
        let manual = Manual::from([
            (
                "item:1".to_owned(),
                vec![seg("基礎拡張", Some("てがき")), seg("マガジン", None)],
            ),
            // 名前が変わった手書きの読みは使わず、LLM の読みを使う
            (
                "item:2".to_owned(),
                vec![seg("古い名前", Some("ふるいなまえ"))],
            ),
        ]);
        let cache = Cache::from([("穴掘り".to_owned(), vec![seg("穴", Some("あな"))])]);
        assert_eq!(missing(&g, &cache, &manual), ["人狼の呪い"]);
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
