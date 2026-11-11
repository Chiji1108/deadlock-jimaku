//! ふりがな（DESIGN.md 6.4）。読みは辞書（Lindera）で付け、文に合わない読みを LLM に直させる。
//!
//! 字幕の文は発話ごとに、辞書の語と読みを渡して、直前の発話を文脈に付けて問い合わせる（`ask`。「人気ないね」の人気(ひとけ)）。
//! 答えが使えない語は辞書の読みのままにする。辞書の漢字名は名前ごとに LLM に読ませてキャッシュする
//! （`assets::readings`。答えを `check` で確かめ、合わなければふりがなを付けない）。

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::access::Access;
use crate::decide;
use crate::llm;
use crate::reading::{self, Furigana, Morpheme};
use crate::types::RubySegment;
use crate::usage::{Meter, Use};

/// 1 文で 2〜3 秒。翻訳と同時に問い合わせる
const TIMEOUT: Duration = Duration::from_secs(8);

/// 測った聞き方（`experiments/furigana_fix.py`）に、文脈と、辞書が読めなかった語を足したもの
const RULES: &str = "\"sentence\" is a Japanese subtitle, \"context\" holds the previous subtitles (oldest first), given only to help you decide readings, \
and \"words\" lists the words of the sentence that contain kanji, each with the reading a dictionary gave, in hiragana (null when the dictionary does not know the word). \
The dictionary reads each word without looking at the sentence, so some readings are wrong for this sentence \
(for example 辛い as つらい when it means spicy, or 一 and 人 as いち and にん when the sentence says ひとり). \
Answer with JSON {\"readings\": [...]}: the correct hiragana reading of each word, in the same order and the same number as \"words\". \
Keep the dictionary's reading unless it is wrong in this sentence. A reading covers only the characters of its word.";

pub struct Model<'a> {
    /// デッドロックの配信として読むか
    pub deadlock: bool,
    pub access: &'a Access,
    pub model: &'a str,
    pub reasoning_effort: Option<&'a str>,
    pub meter: &'a Meter,
}

/// 文のふりがな（つなげると `text` になる）。`context` は直前の確定字幕（古い順）。
/// 漢字を含む語がなければ聞かない。答えが使えない語は、辞書の読みのまま
pub async fn ask(
    http: &reqwest::Client,
    model: &Model<'_>,
    dictionary: &Furigana,
    text: &str,
    context: &[String],
) -> anyhow::Result<Vec<RubySegment>> {
    #[derive(Deserialize)]
    struct Output {
        readings: Vec<String>,
    }
    let mut morphemes = dictionary.morphemes(text);
    let words = words(text, &morphemes);
    if words.is_empty() {
        return Ok(reading::annotate(text, &morphemes, 0, text.len()));
    }
    let schema = json!({
        "type": "object", "additionalProperties": false, "required": ["readings"],
        "properties": { "readings": { "type": "array", "items": { "type": "string" } } },
    });
    let speaker = if model.deadlock {
        "a streamer playing the video game Deadlock"
    } else {
        "a streamer"
    };
    let asked: Vec<_> = words
        .iter()
        .map(|&i| json!({ "text": &text[morphemes[i].start..morphemes[i].end], "reading": morphemes[i].reading }))
        .collect();
    let out: Output = llm::chat_json(
        http,
        llm::Request {
            access: model.access,
            model: model.model,
            reasoning_effort: model.reasoning_effort,
            system: &format!("The input is JSON from {speaker}: {RULES}"),
            user: json!({ "context": context, "sentence": text, "words": asked }).to_string(),
            schema_name: "furigana",
            schema,
            timeout: TIMEOUT,
            meter: model.meter,
            purpose: Use::Furigana,
        },
    )
    .await?;
    if out.readings.len() == words.len() {
        correct(text, &mut morphemes, &words, out.readings);
    } else {
        tracing::warn!("furigana: rejected the answer for {text}");
    }
    Ok(reading::annotate(text, &morphemes, 0, text.len()))
}

/// 読みの候補に付けて見せる、続くかなの数
const TAIL_CHARS: usize = 4;

/// 判定モデル（Jev と同じ形の口。OpenRouter の Perplexity Decider）
pub struct Judge<'a> {
    pub deadlock: bool,
    pub access: &'a Access,
    pub model: &'a str,
    pub meter: &'a Meter,
}

/// 文のふりがな。読みが分かれるところ（辛い、人気、一人）を、辞書の候補から判定モデルに選ばせる。
/// 分かれるところがなければ聞かない。候補の中からしか選べないので、語の範囲を壊す答えは出ない
pub async fn choose(
    http: &reqwest::Client,
    judge: &Judge<'_>,
    dictionary: &Furigana,
    text: &str,
    context: &[String],
) -> anyhow::Result<Vec<RubySegment>> {
    let (mut morphemes, alternatives) = dictionary.alternatives(text);
    if alternatives.is_empty() {
        return Ok(reading::annotate(text, &morphemes, 0, text.len()));
    }
    let name = |i: usize| format!("r{i}");
    let questions: serde_json::Map<String, serde_json::Value> = alternatives
        .iter()
        .enumerate()
        .map(|(i, a)| {
            // 続くかな（送り仮名や助詞）も見せる。語だけだと、活用する語の読みを選べない（「来」を「こ」）
            let tail: String = text[a.end..]
                .chars()
                .take_while(|c| is_kana(*c))
                .take(TAIL_CHARS)
                .collect();
            let criteria: serde_json::Map<String, serde_json::Value> = a
                .options
                .iter()
                .enumerate()
                .map(|(o, option)| {
                    let read = format!("{}{}", reading::read(text, option), to_hiragana(&tail));
                    (format!("o{o}"), read.into())
                })
                .collect();
            let instructions = format!(
                "In the transcript, right after \"{}\", how is \"{}\" read aloud? Each choice is a reading in hiragana. \
                 Choose the reading that fits the meaning of this sentence.",
                &text[..a.start],
                &text[a.start..a.end + tail.len()]
            );
            let question = json!({ "type": "choice", "instructions": instructions, "criteria": criteria });
            (name(i), question)
        })
        .collect();
    let speaker = if judge.deadlock {
        "a Japanese streamer playing the video game Deadlock"
    } else {
        "a Japanese streamer"
    };
    let body = json!({
        "model": judge.model,
        "state": format!(
            "Live commentary from {speaker}, written by speech recognition.\n\
             Previous utterances (context only): {}\nTranscript: {text}",
            json!(context)
        ),
        "questions": questions,
    });
    let asking = decide::Asking {
        access: judge.access,
        model: judge.model,
        meter: judge.meter,
        purpose: Use::Furigana,
    };
    let request = http.post(judge.access.url("/systemone")).json(&body);
    let answers = decide::answers(request, &asking).await?;
    // 後ろから入れ替える（前の語の位置がずれない）
    for (i, a) in alternatives.iter().enumerate().rev() {
        let chosen = decide::choice(&answers, &name(i));
        let chosen =
            chosen.and_then(|c| a.options.get(c.strip_prefix('o')?.parse::<usize>().ok()?));
        if let Some(option) = chosen {
            let from = morphemes.iter().position(|m| m.start == a.start);
            let to = morphemes.iter().position(|m| m.end == a.end);
            if let (Some(from), Some(to)) = (from, to) {
                morphemes.splice(from..=to, option.iter().cloned());
            }
        }
    }
    Ok(reading::annotate(text, &morphemes, 0, text.len()))
}

/// LLM に確かめさせる語（漢字を含む形態素）の位置
fn words(text: &str, morphemes: &[Morpheme]) -> Vec<usize> {
    let kanji = |m: &Morpheme| text[m.start..m.end].chars().any(is_kanji);
    (0..morphemes.len())
        .filter(|&i| kanji(&morphemes[i]))
        .collect()
}

/// LLM の読みを語に入れる。ひらがなでない読みと、語のかなに合わない読み（「避け」に「さける」）は使わず、辞書の読みのまま
fn correct(text: &str, morphemes: &mut [Morpheme], words: &[usize], readings: Vec<String>) {
    for (&i, reading) in words.iter().zip(readings) {
        let word = &text[morphemes[i].start..morphemes[i].end];
        let reading = to_hiragana(&reading);
        let hiragana =
            !reading.is_empty() && reading.chars().all(|c| matches!(c, 'ぁ'..='ゟ' | 'ー'));
        if hiragana && reading::align(word, &reading).is_some() {
            morphemes[i].reading = Some(reading);
        }
    }
}

/// LLM の答えを確かめ、読みを漢字の部分ごとに割り当てる。どれかに合わなければ None
/// - つなげると元の文字列になる
/// - 読みはひらがな（カタカナはひらがなにする）
/// - 読みはかな以外の部分ごとに割り当てる（「取り扱い / とりあつかい」→ 取(と)り扱(あつか)い、
///   「魔呪の罠 / まじゅのわな」→ 魔呪(まじゅ)の罠(わな)、「1番 / いちばん」はまとめて）。
///   割り当てられなければ受け取らない
/// - 漢字には必ず読みがある。漢字のない部分の読み（「OK / おーけー」）は付けない
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
            Some(_) if !s.text.chars().any(is_kanji) => out.push(RubySegment {
                text: s.text,
                ruby: None,
            }),
            Some(r) if !r.chars().all(|c| matches!(c, 'ぁ'..='ゟ' | 'ー')) => return None,
            Some(r) => out.extend(align(&s.text, &r)?),
        }
    }
    Some(out)
}

/// かなの続く部分と、それ以外（漢字・数字など、読みの要る部分）に分け、かなの部分を読みの中で
/// 探して区切る。読みとかなの部分が合わなければ None
pub fn align(text: &str, reading: &str) -> Option<Vec<RubySegment>> {
    let mut runs: Vec<(bool, String)> = Vec::new();
    for c in text.chars() {
        let read = !is_kana(c);
        match runs.last_mut() {
            Some((r, run)) if *r == read => run.push(c),
            _ => runs.push((read, c.to_string())),
        }
    }
    let r: Vec<char> = reading.chars().collect();
    let mut pos = 0;
    let mut out = Vec::new();
    for (i, (read, run)) in runs.iter().enumerate() {
        if !read {
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
        // 読みは、次のかなの部分が読みに現れるところまで（少なくとも 1 文字）
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

/// 読みをそのまま書ける文字。「ヶ」「ヵ」（3ヶ月）は「か」「が」と読むので含めない
fn is_kana(c: char) -> bool {
    matches!(c, 'ぁ'..='ゖ' | 'ァ'..='ヴ' | 'ー')
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

    /// 辞書の読みに、LLM の答えを入れた文のふりがな
    fn corrected(text: &str, readings: &[&str]) -> Vec<RubySegment> {
        let mut morphemes = Furigana::shared().unwrap().morphemes(text);
        let words = words(text, &morphemes);
        assert_eq!(words.len(), readings.len(), "{text}");
        let readings = readings.iter().map(|r| (*r).to_owned()).collect();
        correct(text, &mut morphemes, &words, readings);
        reading::annotate(text, &morphemes, 0, text.len())
    }

    #[test]
    fn the_answer_replaces_the_readings_of_the_dictionary() {
        // 辞書は「からい」を「つらい」、「ひとり」を「いち」「にん」と読む
        assert_eq!(
            corrected("このカレー辛い", &["からい"]),
            [
                seg("このカレー", None),
                seg("辛", Some("から")),
                seg("い", None)
            ]
        );
        assert_eq!(
            corrected("一人で倒した", &["ひと", "り", "たおし"]),
            [
                seg("一", Some("ひと")),
                seg("人", Some("り")),
                seg("で", None),
                seg("倒", Some("たお")),
                seg("した", None)
            ]
        );
    }

    #[test]
    fn an_answer_that_does_not_fit_its_word_keeps_the_dictionary() {
        let dictionary = corrected("攻撃を避けて", &["こうげき", "さけ"]);
        // 送り仮名より先まで読んだ答え、ひらがなでない答え、空の答え
        for wrong in ["さける", "sake", ""] {
            assert_eq!(corrected("攻撃を避けて", &["こうげき", wrong]), dictionary);
        }
        // カタカナはひらがなにして使う
        assert_eq!(
            corrected("攻撃を避けて", &["こうげき", "ヨケ"])[2],
            seg("避", Some("よ"))
        );
    }

    /// 本物の判定モデルに選ばせる: `OPENROUTER_API_KEY=… cargo test -p overlays-core furigana_chooses -- --ignored --nocapture`
    /// `BETA_TEST_API`（開発者のサーバーの根元）と `BETA_TEST_CODE`（ベータのコード）があれば、ベータの中継を通す
    #[tokio::test]
    #[ignore = "calls OpenRouter"]
    async fn furigana_chooses_among_the_readings_of_the_dictionary() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let provider = crate::secrets::Provider::OpenRouter;
        let access = match std::env::var("BETA_TEST_API") {
            Ok(api) => Access::gift(provider, std::env::var("BETA_TEST_CODE").unwrap(), &api),
            Err(_) => Access::key(provider, std::env::var("OPENROUTER_API_KEY").unwrap()),
        };
        let meter = Meter::new(std::env::temp_dir().join("overlays-furigana-test-usage.json"));
        let judge = Judge {
            deadlock: true,
            access: &access,
            model: "perplexity/pplx-decider-v1.2",
            meter: &meter,
        };
        for (text, expected) in [
            (
                "このカレーめっちゃ辛いんだけど",
                "このカレーめっちゃからいんだけど",
            ),
            ("一人で三人倒した", "ひとりでさんにんたおした"),
            ("火力が上がってる", "かりょくがあがってる"),
        ] {
            let dictionary = Furigana::shared().unwrap();
            let segments = choose(crate::http(), &judge, dictionary, text, &[])
                .await
                .unwrap();
            let reading: String = segments
                .iter()
                .map(|s| s.ruby.as_deref().unwrap_or(&s.text))
                .collect();
            println!("{text} → {reading}");
            assert_eq!(reading, expected);
        }
    }

    /// 本物の OpenAI に聞く: `OPENAI_API_KEY=… cargo test -p overlays-core furigana_fixes -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "calls OpenAI"]
    async fn furigana_fixes_the_readings_in_context() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let key = std::env::var("OPENAI_API_KEY").unwrap();
        let access = Access::key(crate::secrets::Provider::OpenAi, key);
        let meter = Meter::new(std::env::temp_dir().join("overlays-furigana-test-usage.json"));
        for name in ["gpt-6.1-terra", "gpt-6-luna"] {
            let model = Model {
                deadlock: true,
                access: &access,
                model: name,
                reasoning_effort: Some("none"),
                meter: &meter,
            };
            let read = |text: &'static str, context: &'static [&'static str]| {
                let model = &model;
                async move {
                    let context: Vec<String> = context.iter().map(|c| (*c).to_owned()).collect();
                    let started = std::time::Instant::now();
                    let dictionary = Furigana::shared().unwrap();
                    let segments = ask(crate::http(), model, dictionary, text, &context)
                        .await
                        .unwrap();
                    let reading: String = segments
                        .iter()
                        .map(|s| s.ruby.as_deref().unwrap_or(&s.text))
                        .collect();
                    println!("{name} {:?} {text} → {reading}", started.elapsed());
                    reading
                }
            };
            assert_eq!(
                read("このカレーめっちゃ辛いんだけど", &[]).await,
                "このカレーめっちゃからいんだけど"
            );
            assert_eq!(
                read("一人で三人倒した", &[]).await,
                "ひとりでさんにんたおした"
            );
            // 文だけでは決まらない読みは、前の発話で決まる
            assert_eq!(
                read("人気ないね", &["このレーン誰もいないな"]).await,
                "ひとけないね"
            );
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
        // 数字やヶは漢字と一緒に読む
        assert_eq!(
            align("1番", "いちばん"),
            Some(vec![seg("1番", Some("いちばん"))])
        );
        assert_eq!(
            align("3ヶ月", "さんかげつ"),
            Some(vec![seg("3ヶ月", Some("さんかげつ"))])
        );
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
    fn check_drops_readings_of_kanji_free_parts() {
        assert_eq!(
            check(
                "OKだね",
                vec![seg("OK", Some("おーけー")), seg("だね", None)]
            ),
            Some(vec![seg("OK", None), seg("だね", None)])
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
