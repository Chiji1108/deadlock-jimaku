//! ふりがな（DESIGN.md 6.4）。読みは LLM（GPT-6.1 Terra）に文脈ごと決めさせる。
//!
//! 字幕の文は発話ごとに問い合わせる（`ask`）。辞書の漢字名は名前ごとに問い合わせてキャッシュする
//! （`assets::readings`）。どちらも答えを `check` で確かめ、合わなければふりがなを付けない
//! （ふりがなは補助なので、まれに付かなくても字幕としては困らない）。

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::llm;
use crate::types::RubySegment;

/// 1 文で 2〜3 秒。翻訳と同時に問い合わせる
const TIMEOUT: Duration = Duration::from_secs(8);

const SYSTEM: &str = "This is a Japanese subtitle of a streamer playing the video game Deadlock. \
Split it into words and give the hiragana reading of each word that contains kanji, reading it the way it is meant in context. \
A kanji compound is one segment with the reading of the whole word; never split a compound into single kanji. \
Keep okurigana, kana and punctuation in their own segments with ruby null. Joining the segments' text must give the sentence exactly.";

pub struct Model<'a> {
    pub api_key: &'a str,
    pub model: &'a str,
    pub reasoning_effort: Option<&'a str>,
}

/// 文のふりがな。答えが `check` に通らなければ None（ふりがなを付けない）
pub async fn ask(
    http: &reqwest::Client,
    model: &Model<'_>,
    text: &str,
) -> anyhow::Result<Option<Vec<RubySegment>>> {
    #[derive(Deserialize)]
    struct Output {
        segments: Vec<RubySegment>,
    }
    let segment = json!({
        "type": "object", "additionalProperties": false, "required": ["text", "ruby"],
        "properties": { "text": { "type": "string" }, "ruby": { "type": ["string", "null"] } },
    });
    let schema = json!({
        "type": "object", "additionalProperties": false, "required": ["segments"],
        "properties": { "segments": { "type": "array", "items": segment } },
    });
    let out: Output = llm::chat_json(
        http,
        llm::Request {
            api_key: model.api_key,
            model: model.model,
            reasoning_effort: model.reasoning_effort,
            system: SYSTEM,
            user: text.to_owned(),
            schema_name: "furigana",
            schema,
            timeout: TIMEOUT,
        },
    )
    .await?;
    let checked = check(text, out.segments);
    if checked.is_none() {
        tracing::warn!("furigana: rejected the answer for {text}");
    }
    Ok(checked)
}

/// LLM の答えを確かめ、読みを漢字の部分ごとに割り当てる。どれかに合わなければ None
/// - つなげると元の文字列になる
/// - 読みはひらがな（カタカナはひらがなにする）
/// - 読みは漢字の部分ごとに割り当てる（「取り扱い / とりあつかい」→ 取(と)り扱(あつか)い、
///   「魔呪の罠 / まじゅのわな」→ 魔呪(まじゅ)の罠(わな)）。割り当てられなければ受け取らない
/// - 漢字には必ず読みがある
pub fn check(text: &str, segments: Vec<RubySegment>) -> Option<Vec<RubySegment>> {
    let joined: String = segments.iter().map(|s| s.text.as_str()).collect();
    if joined != text {
        return None;
    }
    let mut out = Vec::new();
    for s in segments {
        match s.ruby.as_deref().map(to_hiragana) {
            None if s.text.chars().any(is_kanji) => return None,
            None => out.push(s),
            Some(r) if !r.chars().all(|c| matches!(c, 'ぁ'..='ゟ' | 'ー')) => return None,
            Some(r) => out.extend(align(&s.text, &r)?),
        }
    }
    Some(out)
}

/// 漢字の続く部分と、それ以外（かななど）の部分に分け、かなの部分を読みの中で探して区切る。
/// 読みとかなの部分が合わなければ None
pub fn align(text: &str, reading: &str) -> Option<Vec<RubySegment>> {
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
            out.push(RubySegment {
                text: run.clone(),
                ruby: None,
            });
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
        out.push(RubySegment {
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

pub fn is_kanji(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '々')
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
    fn align_moves_okurigana_and_kana_out() {
        assert_eq!(
            align("強い", "つよい"),
            Some(vec![seg("強", Some("つよ")), seg("い", None)])
        );
        assert_eq!(
            align("取り扱い", "とりあつかい"),
            Some(vec![
                seg("取", Some("と")),
                seg("り", None),
                seg("扱", Some("あつか")),
                seg("い", None)
            ])
        );
        // かなの部分が読みに現れない
        assert_eq!(align("取り扱い", "とあつかい"), None);
    }

    #[test]
    fn check_splits_okurigana_and_normalizes_katakana() {
        assert_eq!(
            check(
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
        assert_eq!(
            check(
                "一人で行く。",
                vec![
                    seg("一人", Some("ひとり")),
                    seg("で", None),
                    seg("行く", Some("いく")),
                    seg("。", None)
                ]
            ),
            Some(vec![
                seg("一人", Some("ひとり")),
                seg("で", None),
                seg("行", Some("い")),
                seg("く", None),
                seg("。", None)
            ])
        );
    }

    #[test]
    fn check_rejects_bad_answers() {
        // つなげても元にならない（LLM が別の文を作った）
        assert_eq!(check("人狼", vec![seg("斧", Some("おの"))]), None);
        // 読みがかなでない、区切り記号が入っている
        assert_eq!(check("人狼", vec![seg("人狼", Some("jinrou"))]), None);
        assert_eq!(check("人狼", vec![seg("人狼", Some("じん・ろう"))]), None);
        // 漢字に読みがない
        assert_eq!(
            check(
                "狩猟罠",
                vec![seg("狩猟", Some("しゅりょう")), seg("罠", None)]
            ),
            None
        );
        // 読みのかなの部分が合わない
        assert_eq!(
            check("魔呪の罠", vec![seg("魔呪の罠", Some("まじゅがわな"))]),
            None
        );
    }

    #[test]
    fn check_aligns_readings_across_kana() {
        assert_eq!(
            check("魔呪の罠", vec![seg("魔呪の罠", Some("まじゅのわな"))]),
            Some(vec![
                seg("魔呪", Some("まじゅ")),
                seg("の", None),
                seg("罠", Some("わな"))
            ])
        );
        assert_eq!(
            check("いすゞ", vec![seg("いすゞ", None)]),
            Some(vec![seg("いすゞ", None)])
        );
    }
}
