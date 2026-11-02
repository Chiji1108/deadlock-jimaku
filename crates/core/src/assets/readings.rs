//! 辞書の漢字名の読み（DESIGN.md 8.2）。
//!
//! 辞書は形態素解析の読みで保存し、読み込むたびに LLM の読み（名前ごとのキャッシュ）を重ねる。
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
use crate::types::{Glossary, RubySegment, Token};

/// 35 語で 20 秒ほど
const TIMEOUT: Duration = Duration::from_secs(120);

pub struct ReadingModel {
    pub api_key: String,
    pub model: String,
}

/// 名前 → 読み
pub type Cache = BTreeMap<String, Vec<RubySegment>>;

/// 壊れていたら脇に退けて空から始める（黙って上書きしない）
pub async fn load_cache(path: &Path) -> Cache {
    let Ok(bytes) = tokio::fs::read(path).await else {
        return Cache::new();
    };
    match serde_json::from_slice(&bytes) {
        Ok(cache) => cache,
        Err(e) => {
            let aside = path.with_extension("json.broken");
            tracing::warn!(
                "readings cache is broken ({e}); moved to {}",
                aside.display()
            );
            let _ = tokio::fs::rename(path, aside).await;
            Cache::new()
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

/// 読みが必要なエンティティの日本語名。手で読みを書いたもの（`manual`）は除く
fn kanji_names<'a>(
    glossary: &'a Glossary,
    manual: &'a BTreeSet<String>,
) -> impl Iterator<Item = (&'a String, &'a String)> {
    glossary
        .entities
        .iter()
        .filter(|(r, _)| !manual.contains(*r))
        .filter_map(|(r, e)| Some((r, e.names.get("ja")?)))
        .filter(|(_, name)| name.chars().any(furigana::is_kanji))
}

/// キャッシュにある読みを辞書に重ねる
pub fn apply(glossary: &mut Glossary, cache: &Cache, manual: &BTreeSet<String>) {
    let found: Vec<(String, Vec<RubySegment>)> = kanji_names(glossary, manual)
        .filter_map(|(r, name)| Some((r.clone(), cache.get(name)?.clone())))
        .collect();
    for (reference, ruby) in found {
        if let Some(e) = glossary.entities.get_mut(&reference) {
            e.ruby = Some(ruby);
        }
    }
}

/// まだ問い合わせていない名前
pub fn missing(glossary: &Glossary, cache: &Cache, manual: &BTreeSet<String>) -> Vec<String> {
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
            let checked = validate(name, segments);
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

/// - つなげると名前になる
/// - 読みはひらがな（カタカナはひらがなにする）
/// - 送り仮名は外に出す（「呪い / のろい」→「呪(のろ)」「い」）
/// - ルビの付く部分は漢字だけ、漢字には必ずルビがある（「魔呪の狩猟罠」全体に 1 つのルビ、のようなものは受け取らない）
fn validate(name: &str, segments: Vec<RubySegment>) -> Option<Vec<RubySegment>> {
    let joined: String = segments.iter().map(|s| s.text.as_str()).collect();
    if joined != name {
        return None;
    }
    let mut out = Vec::new();
    for s in segments {
        let ruby = s.ruby.as_deref().map(furigana::to_hiragana);
        if ruby
            .as_deref()
            .is_some_and(|r| !r.chars().all(|c| matches!(c, 'ぁ'..='ゖ' | 'ー')))
        {
            return None;
        }
        for t in furigana::ruby_tokens(&s.text, ruby.as_deref()) {
            let Token::Text { text, ruby } = t else {
                return None;
            };
            let ok = match &ruby {
                Some(_) => text.chars().all(furigana::is_kanji),
                None => !text.chars().any(furigana::is_kanji),
            };
            if !ok {
                return None;
            }
            out.push(RubySegment { text, ruby });
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str, ruby: Option<&str>) -> RubySegment {
        RubySegment {
            text: text.into(),
            ruby: ruby.map(Into::into),
        }
    }

    #[test]
    fn validate_splits_okurigana_and_normalizes_katakana() {
        assert_eq!(
            validate(
                "人狼の呪い",
                vec![
                    seg("人狼", Some("ジンロウ")),
                    seg("の", None),
                    seg("呪い", Some("のろい"))
                ]
            ),
            Some(vec![
                seg("人狼", Some("じんろう")),
                seg("の", None),
                seg("呪", Some("のろ")),
                seg("い", None)
            ])
        );
    }

    #[test]
    fn validate_rejects_bad_answers() {
        // つなげても名前にならない（LLM が別の文を作った）
        assert_eq!(validate("人狼", vec![seg("斧", Some("おの"))]), None);
        // 読みがかなでない、区切り記号が入っている
        assert_eq!(validate("人狼", vec![seg("人狼", Some("jinrou"))]), None);
        assert_eq!(
            validate("人狼", vec![seg("人狼", Some("じん・ろう"))]),
            None
        );
        // 漢字に読みがない
        assert_eq!(
            validate(
                "狩猟罠",
                vec![seg("狩猟", Some("しゅりょう")), seg("罠", None)]
            ),
            None
        );
        // 助詞をはさんだ全体に 1 つのルビ
        assert_eq!(
            validate("魔呪の罠", vec![seg("魔呪の罠", Some("まじゅのわな"))]),
            None
        );
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
        let manual = BTreeSet::from(["item:1".to_owned()]);
        let cache = Cache::from([
            (
                "人狼の呪い".to_owned(),
                vec![seg("人狼", Some("じんろう")), seg("の呪い", None)],
            ),
            ("基礎拡張マガジン".to_owned(), vec![seg("x", Some("x"))]),
        ]);
        apply(&mut g, &cache, &manual);
        assert_eq!(
            g.entities["ability:1"].ruby.as_ref().unwrap()[0]
                .ruby
                .as_deref(),
            Some("じんろう")
        );
        // 手で書いた読みは上書きしない。キャッシュにない名前はそのまま
        assert!(g.entities["item:1"].ruby.is_none());
        assert!(g.entities["item:2"].ruby.is_none());
    }

    #[test]
    fn missing_skips_cached_manual_and_kana_names() {
        let g = glossary();
        let manual = BTreeSet::from(["item:1".to_owned()]);
        let cache = Cache::from([("穴掘り".to_owned(), vec![seg("穴", Some("あな"))])]);
        assert_eq!(missing(&g, &cache, &manual), ["人狼の呪い"]);
    }

    #[tokio::test]
    async fn broken_cache_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        tokio::fs::write(&path, b"{ broken").await.unwrap();
        assert!(load_cache(&path).await.is_empty());
        assert!(dir.path().join("cache.json.broken").exists());
    }
}
