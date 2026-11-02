//! 辞書が知らない語（造語の多いゲーム用語）の読みを LLM で付ける（DESIGN.md 8.2）。
//!
//! 辞書を作るときに 1 回だけ問い合わせ、名前ごとにキャッシュする。名前はパッチをまたいでも
//! ほとんど変わらないので、次からは新しい名前だけを問い合わせる。

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::furigana;
use crate::llm;
use crate::types::{RubySegment, Token};

/// 推論するモデルを使うので長めにする（35 語で 35 秒ほど）
const TIMEOUT: Duration = Duration::from_secs(180);

pub struct ReadingModel {
    pub api_key: String,
    pub model: String,
}

/// 名前 → 形態素解析での読み（LLM の答えと比べるのに使う）
pub type Fallbacks = BTreeMap<String, Vec<RubySegment>>;

/// 名前の読みを返す。キャッシュになければ LLM に問い合わせる。
/// LLM の答えが検証に通らなかった名前は含まない（形態素解析の読みのままにする）。
/// Err は問い合わせ自体が失敗したときだけ（次の起動で問い直す）
pub async fn readings(
    model: &ReadingModel,
    cache_path: &Path,
    names: &Fallbacks,
) -> anyhow::Result<BTreeMap<String, Vec<RubySegment>>> {
    let mut cache: BTreeMap<String, Vec<RubySegment>> = std::fs::read(cache_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let missing: Vec<&String> = names.keys().filter(|n| !cache.contains_key(*n)).collect();
    if !missing.is_empty() {
        let found = ask(model, &missing, names).await?;
        tracing::info!("readings: {}/{} from LLM", found.len(), missing.len());
        cache.extend(found);
        if let Err(e) = save(cache_path, &cache).await {
            tracing::warn!("readings cache: {e:#}");
        }
    }
    Ok(names
        .keys()
        .filter_map(|n| Some((n.clone(), cache.get(n)?.clone())))
        .collect())
}

async fn ask(
    model: &ReadingModel,
    names: &[&String],
    fallbacks: &Fallbacks,
) -> anyhow::Result<BTreeMap<String, Vec<RubySegment>>> {
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
        &reqwest::Client::new(),
        llm::Request {
            api_key: &model.api_key,
            model: &model.model,
            system: SYSTEM,
            user: serde_json::to_string(names)?,
            schema_name: "readings",
            schema,
            reasoning_effort: Some("low"),
            timeout: TIMEOUT,
        },
    )
    .await?;
    Ok(out
        .names
        .into_iter()
        .filter(|n| names.contains(&&n.name))
        .filter_map(|n| {
            let fallback = fallbacks.get(&n.name)?;
            let checked = validate(&n.name, n.segments, fallback);
            if checked.is_none() {
                tracing::warn!("readings: rejected the LLM reading of {}", n.name);
            }
            Some((n.name.clone(), checked?))
        })
        .collect())
}

const SYSTEM: &str = "These are Japanese names of heroes' abilities and items in the video game Deadlock (Japanese localization). \
For each name, split it into words and give the hiragana reading of each word that contains kanji. \
A kanji compound is one segment with the reading of the whole word (退避 → たいひ, 人狼 → じんろう); never split a compound into single kanji. \
Keep okurigana and kana in their own segments with ruby null. Joining the segments' text must give the name exactly. \
Many names are coined compounds; read them the way a Japanese gamer would read them aloud.";

/// つなげると名前になり、読みがかなであること。送り仮名は外に出す（「呪い / のろい」→「呪(のろ)」「い」）。
/// 複合語を 1 文字ずつに分けて読んだもの（「退(た)避(ひ)」）は読みを誤りやすいので、
/// 形態素解析と読みが一致するとき（「穴(あな)掘(ほ)り」）だけ受け取る
fn validate(
    name: &str,
    segments: Vec<RubySegment>,
    fallback: &[RubySegment],
) -> Option<Vec<RubySegment>> {
    let joined: String = segments.iter().map(|s| s.text.as_str()).collect();
    if joined != name {
        return None;
    }
    let tokens: Vec<Token> = segments
        .into_iter()
        .flat_map(|s| furigana::ruby_tokens(&s.text, s.ruby.as_deref()))
        .collect();
    if furigana::is_uncertain(&tokens) && reading(&tokens) != reading_of(fallback) {
        return None;
    }
    let out: Vec<RubySegment> = tokens
        .into_iter()
        .filter_map(|t| match t {
            Token::Text { text, ruby } => Some(RubySegment { text, ruby }),
            Token::Entity { .. } => None,
        })
        .collect();
    out.iter().any(|s| s.ruby.is_some()).then_some(out)
}

/// 名前全体の読み（ルビのない部分はそのまま）
fn reading(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|t| match t {
            Token::Text { ruby: Some(r), .. } => r.as_str(),
            Token::Text { text, .. } => text.as_str(),
            Token::Entity { .. } => "",
        })
        .collect()
}

fn reading_of(segments: &[RubySegment]) -> String {
    segments
        .iter()
        .map(|s| s.ruby.as_deref().unwrap_or(&s.text))
        .collect()
}

async fn save(path: &Path, cache: &BTreeMap<String, Vec<RubySegment>>) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(path.parent().expect("has parent")).await?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, serde_json::to_vec_pretty(cache)?).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
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
    fn validate_splits_okurigana_and_rejects_mismatches() {
        assert_eq!(
            validate(
                "人狼の呪い",
                vec![
                    seg("人狼", Some("じんろう")),
                    seg("の", None),
                    seg("呪い", Some("のろい"))
                ],
                &[]
            ),
            Some(vec![
                seg("人狼", Some("じんろう")),
                seg("の", None),
                seg("呪", Some("のろ")),
                seg("い", None)
            ])
        );
        // つなげても名前にならない（LLM が別の文を作った）
        assert_eq!(validate("人狼", vec![seg("斧", Some("おの"))], &[]), None);
        // 読みがかなでない
        assert_eq!(
            validate("人狼", vec![seg("人狼", Some("jinrou"))], &[]),
            None
        );
        // 複合語を 1 文字ずつに分けた。形態素解析と読みが違えば受け取らない
        let split = vec![seg("退", Some("た")), seg("避", Some("ひ"))];
        assert_eq!(
            validate("退避", split, &[seg("退避", Some("たいひ"))]),
            None
        );
        // 1 文字ずつでも、形態素解析と読みが一致すれば受け取る
        let dig = vec![seg("穴", Some("あな")), seg("掘り", Some("ほり"))];
        let lindera = [
            seg("穴", Some("あな")),
            seg("掘", Some("ほ")),
            seg("り", None),
        ];
        assert!(validate("穴掘り", dig, &lindera).is_some());
    }

    #[tokio::test]
    async fn cached_names_do_not_call_the_llm() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let cached = BTreeMap::from([("人狼".to_owned(), vec![seg("人狼", Some("じんろう"))])]);
        save(&path, &cached).await.unwrap();
        // API キーが偽物なので、問い合わせれば失敗する。キャッシュだけで返ることを確かめる
        let model = ReadingModel {
            api_key: "invalid".into(),
            model: "model".into(),
        };
        let names = BTreeMap::from([(
            "人狼".to_owned(),
            vec![seg("人", Some("ひと")), seg("狼", Some("おおかみ"))],
        )]);
        let got = readings(&model, &path, &names).await.unwrap();
        assert_eq!(got, cached);
    }
}
