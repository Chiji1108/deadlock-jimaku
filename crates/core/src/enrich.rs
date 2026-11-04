//! 確定した字幕に、用語・ふりがな・翻訳を付ける（DESIGN.md 9 章）。
//!
//! - 用語と翻訳: LLM に辞書の用語の一覧（公式名とスキルの持ち主・番号）を渡し、字幕を「ふつうの文字」と
//!   「用語」の区切りに分けさせ、同じ問い合わせで訳させる。略称・言い間違い・番号で呼んだスキルも文脈から決まる。
//!   訳文の用語は `⟦hero:13⟧` のように参照で書かせ、各言語の公式名は描画時に辞書から出す
//! - ふりがな: LLM に文ごとに聞き（`furigana`）、用語と同時に問い合わせる
//!
//! 答えは確かめてから使う。区切りをつなげて字幕にならなければ用語を付けず、辞書にない参照は捨てる。
//! ふりがなは文字の位置で区切りに当てはめる（`source_tokens`）。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::furigana;
use crate::lang;
use crate::llm;
use crate::openai::Access;
use crate::types::{Enriched, EntityKind, Glossary, LangCode, RubySegment, Token};
use crate::usage::{Meter, Use};

const TIMEOUT: Duration = Duration::from_secs(8);

pub struct Enricher {
    http: reqwest::Client,
    /// OpenAI への送り先と鍵
    access: Access,
    model: String,
    /// ふりがなのモデル（辞書の名前の読みと同じ。DESIGN.md 8.2）
    readings_model: String,
    reasoning_effort: Option<String>,
    source_lang: LangCode,
    targets: Vec<LangCode>,
    meter: Arc<Meter>,
    /// 辞書の版ごとに作り直す system（用語の一覧を含む固定の先頭部分なので、プロンプトキャッシュが効く）
    system: Mutex<Option<(Option<GlossaryKey>, Arc<str>)>>,
}

impl Enricher {
    pub fn new(
        access: Access,
        model: String,
        readings_model: String,
        reasoning_effort: Option<String>,
        source_lang: LangCode,
        targets: Vec<LangCode>,
        meter: Arc<Meter>,
    ) -> Self {
        let targets = lang::normalize(targets.into_iter().filter(|t| *t != source_lang));
        Self {
            http: crate::http().clone(),
            access,
            model,
            readings_model,
            reasoning_effort,
            source_lang,
            targets,
            meter,
            system: Mutex::new(None),
        }
    }

    /// `context` は直前の確定字幕（古い順）。用語と翻訳に失敗したら、原文（とふりがな）だけ返す。
    /// 雑音（咳やフィラーだけ）はバッチの文字起こしが空になるので、ここでは判定しない
    pub async fn enrich(
        &self,
        text: &str,
        context: &[String],
        glossary: Option<&Glossary>,
        furigana: bool,
    ) -> Enriched {
        let (ruby, analysis) = tokio::join!(
            self.furigana(text, context, furigana),
            self.analyze(text, context, glossary)
        );
        let (spans, translations) = match analysis {
            Some(a) => (
                a.spans(text, glossary),
                a.translations(&self.targets, glossary),
            ),
            None => (vec![Span::Text(text.to_owned())], BTreeMap::new()),
        };
        Enriched {
            source: source_tokens(&spans, ruby.as_deref().unwrap_or_default()),
            translations,
        }
    }

    /// ふりがなを出す設定で、日本語で漢字を含む文だけ聞く。失敗したらふりがななし
    async fn furigana(
        &self,
        text: &str,
        context: &[String],
        enabled: bool,
    ) -> Option<Vec<RubySegment>> {
        if !enabled || self.source_lang != "ja" || !text.chars().any(furigana::is_kanji) {
            return None;
        }
        let model = furigana::Model {
            access: &self.access,
            model: &self.readings_model,
            reasoning_effort: self.reasoning_effort.as_deref(),
            meter: &self.meter,
        };
        furigana::ask(&self.http, &model, text, context)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!("furigana: {e:#}");
                None
            })
    }

    /// 用語と翻訳。辞書も翻訳先もなければ聞くことがない
    async fn analyze(
        &self,
        text: &str,
        context: &[String],
        glossary: Option<&Glossary>,
    ) -> Option<Analysis> {
        if glossary.is_none() && self.targets.is_empty() {
            return None;
        }
        let user = json!({ "context": context, "transcript": text });
        llm::chat_json(
            &self.http,
            llm::Request {
                access: &self.access,
                model: &self.model,
                reasoning_effort: self.reasoning_effort.as_deref(),
                system: &self.system(glossary),
                user: user.to_string(),
                schema_name: "subtitle",
                schema: self.schema(),
                timeout: TIMEOUT,
                meter: &self.meter,
                purpose: Use::Translate,
            },
        )
        .await
        .inspect_err(|e| tracing::warn!("terms and translation: {e:#}"))
        .ok()
    }

    fn system(&self, glossary: Option<&Glossary>) -> Arc<str> {
        // 同じ版でも、言語を足して作り直した辞書は名前が違う
        let version = glossary.map(|g| (g.client_version, g.languages.clone()));
        let mut cached = self.system.lock().unwrap();
        match &*cached {
            Some((v, system)) if *v == version => system.clone(),
            _ => {
                let system: Arc<str> = system_prompt(&self.source_lang, glossary).into();
                *cached = Some((version, system.clone()));
                system
            }
        }
    }

    fn schema(&self) -> Value {
        let segments = json!({
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["text", "ref"],
                "properties": {
                    "text": { "type": "string" },
                    "ref": { "type": ["string", "null"] },
                },
            },
        });
        if self.targets.is_empty() {
            return json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["segments"],
                "properties": { "segments": segments },
            });
        }
        let translations: serde_json::Map<String, Value> = self
            .targets
            .iter()
            .map(|t| (t.clone(), json!({ "type": "string" })))
            .collect();
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["segments", "translations"],
            "properties": {
                "segments": segments,
                "translations": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": self.targets,
                    "properties": translations,
                },
            },
        })
    }
}

/// どの辞書から作った system か（版と、名前のある言語）
type GlossaryKey = (u32, Vec<LangCode>);

/// LLM の答え
#[derive(Debug, Deserialize)]
struct Analysis {
    /// つなげると字幕になる区切り。用語なら参照
    segments: Vec<Segment>,
    /// 訳文。用語は `⟦参照⟧`。翻訳先がなければ聞かない
    #[serde(default)]
    translations: BTreeMap<LangCode, String>,
}

#[derive(Debug, Deserialize)]
struct Segment {
    text: String,
    #[serde(rename = "ref")]
    reference: Option<String>,
}

impl Analysis {
    /// 原文の区切り。辞書にない参照はふつうの文字にして、隣の文字とまとめる。同じ用語が続けば 1 つにする。
    /// つなげても字幕にならなければ、どこが用語か分からないので用語を付けない
    fn spans(&self, text: &str, glossary: Option<&Glossary>) -> Vec<Span> {
        let joined: String = self.segments.iter().map(|s| s.text.as_str()).collect();
        if joined != text {
            tracing::warn!("segments do not join to the transcript: {joined}");
            return vec![Span::Text(text.to_owned())];
        }
        let mut spans = Vec::new();
        for s in self.segments.iter().filter(|s| !s.text.is_empty()) {
            match (
                s.reference.as_ref().filter(|r| known(glossary, r)),
                spans.last_mut(),
            ) {
                (Some(r), Some(Span::Entity { reference, spoken })) if reference == r => {
                    spoken.push_str(&s.text);
                }
                (Some(r), _) => spans.push(Span::Entity {
                    reference: r.clone(),
                    spoken: s.text.clone(),
                }),
                (None, Some(Span::Text(t))) => t.push_str(&s.text),
                (None, _) => spans.push(Span::Text(s.text.clone())),
            }
        }
        spans
    }

    /// 頼んでいない言語は捨て、訳文の `⟦参照⟧` をエンティティに戻す。
    /// 戻すのは原文の区切りで見つけた用語だけ（原文にない用語を訳文にだけ出さない）
    fn translations(
        self,
        targets: &[LangCode],
        glossary: Option<&Glossary>,
    ) -> BTreeMap<LangCode, Vec<Token>> {
        let terms: Vec<&str> = self
            .segments
            .iter()
            .filter_map(|s| s.reference.as_deref())
            .filter(|r| known(glossary, r))
            .collect();
        self.translations
            .iter()
            .filter(|(lang, _)| targets.contains(lang))
            .map(|(lang, text)| (lang.clone(), resolve(text, &terms)))
            .collect()
    }
}

fn known(glossary: Option<&Glossary>, reference: &str) -> bool {
    glossary.is_some_and(|g| g.entities.contains_key(reference))
}

/// 訳文の `⟦参照⟧` をエンティティにする。`terms` にない参照は捨て、そのために空いた空白を詰める
fn resolve(text: &str, terms: &[&str]) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut buf = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('⟦') {
        let Some(len) = rest[start..].find('⟧') else {
            break;
        };
        let end = start + len;
        buf.push_str(&rest[..start]);
        let reference = rest[start + '⟦'.len_utf8()..end].trim();
        let after = &rest[end + '⟧'.len_utf8()..];
        if terms.contains(&reference) {
            push_text(&mut tokens, std::mem::take(&mut buf));
            tokens.push(Token::Entity {
                reference: reference.to_owned(),
                spoken: None,
                ruby: None,
            });
            rest = after;
        } else {
            // 捨てた用語の所有格（⟦x⟧'s）も残さない
            let after = ["'s", "’s"]
                .iter()
                .find_map(|s| after.strip_prefix(s))
                .unwrap_or(after);
            if buf.ends_with(' ') && after.starts_with(' ') {
                buf.pop();
            }
            rest = after;
        }
    }
    buf.push_str(rest);
    push_text(&mut tokens, buf);
    tokens
}

fn push_text(tokens: &mut Vec<Token>, text: String) {
    if !text.is_empty() {
        tokens.push(Token::Text { text, ruby: None });
    }
}

/// 原文のトークン。文のふりがな（`ruby`、つなげると原文になる）を、文字の位置で区切りに当てはめる。
/// 区切りをまたぐ部分はふりがなを付けない
fn source_tokens(spans: &[Span], ruby: &[RubySegment]) -> Vec<Token> {
    let mut placed = Vec::new();
    let mut pos = 0;
    for seg in ruby {
        let len = seg.text.chars().count();
        placed.push((pos, pos + len, seg));
        pos += len;
    }
    let mut start = 0;
    spans
        .iter()
        .flat_map(|span| {
            let text = match span {
                Span::Text(text) => text,
                Span::Entity { spoken, .. } => spoken,
            };
            let end = start + text.chars().count();
            let segments = cover(text, start, &placed);
            start = end;
            match span {
                Span::Text(_) => segments
                    .into_iter()
                    .map(|s| Token::Text {
                        text: s.text,
                        ruby: s.ruby,
                    })
                    .collect(),
                Span::Entity { reference, spoken } => vec![Token::Entity {
                    reference: reference.clone(),
                    spoken: Some(spoken.clone()),
                    ruby: segments
                        .iter()
                        .any(|s| s.ruby.is_some())
                        .then_some(segments),
                }],
            }
        })
        .collect()
}

/// `text`（原文の `offset` 文字目から）を、中に収まるふりがなの部分と、読みのない部分に分ける
fn cover(text: &str, offset: usize, placed: &[(usize, usize, &RubySegment)]) -> Vec<RubySegment> {
    let chars: Vec<char> = text.chars().collect();
    let end = offset + chars.len();
    let mut out: Vec<RubySegment> = Vec::new();
    let plain = |out: &mut Vec<RubySegment>, from: usize, to: usize| {
        if from >= to {
            return;
        }
        let part: String = chars[from - offset..to - offset].iter().collect();
        match out.last_mut() {
            Some(last) if last.ruby.is_none() => last.text.push_str(&part),
            _ => out.push(RubySegment {
                text: part,
                ruby: None,
            }),
        }
    };
    let mut pos = offset;
    for &(s, e, seg) in placed {
        if s < offset || e > end || seg.ruby.is_none() {
            continue;
        }
        plain(&mut out, pos, s);
        out.push(seg.clone());
        pos = e;
    }
    plain(&mut out, pos, end);
    out
}

#[derive(Debug, PartialEq)]
enum Span {
    Text(String),
    Entity { reference: String, spoken: String },
}

/// 役割とルールと用語の一覧。用語は 1 行ずつ「参照 | 種類 | 話す言語の名前 | 英語名 | スキルの持ち主と番号」
fn system_prompt(source_lang: &str, glossary: Option<&Glossary>) -> String {
    let lang_label = lang::find(source_lang).map_or(source_lang, |l| l.label);
    // デッドロックモードを切ったときは、ゲームの話として扱わない
    if glossary.is_none() {
        return format!(
            "You process live commentary from a streamer who speaks {lang_label}. Each request has \"transcript\" \
             (one utterance from speech recognition, which may mishear words) and \"context\" \
             (the previous utterances, for reference only; do not split or translate them).\n\
             \n\
             Rules:\n\
             1. \"segments\": return the whole transcript as a single segment, exactly as given, with ref null.\n\
             2. \"translations\": translate only the transcript into each requested language, naturally and casually.\n"
        );
    }
    let mut lines = Vec::new();
    for (reference, e) in glossary.map(|g| &g.entities).into_iter().flatten() {
        let kind = match e.kind {
            EntityKind::Hero => "hero",
            EntityKind::Item => "item",
            EntityKind::Ability => "ability",
        };
        // 列を揃える（名前がなければ英語名）
        let en = e.names.get("en").map_or("", String::as_str);
        let name = e.names.get(source_lang).map_or(en, String::as_str);
        let mut line = format!("{reference} | {kind} | {name} | {en}");
        if let (Some(hero), Some(slot)) = (&e.hero, e.ability_slot) {
            let owner = glossary
                .and_then(|g| g.entities.get(hero))
                .and_then(|h| h.names.get("en"))
                .map_or("?", String::as_str);
            line += &format!(" | ability {slot} of {owner} ({hero})");
        }
        lines.push(line);
    }
    format!(
        "You process live commentary from a streamer playing the video game Deadlock. \
         The streamer speaks {lang_label}. Each request has \"transcript\" (one utterance from speech recognition, \
         which may mishear words; the streamer may also mispronounce names) and \"context\" \
         (the previous utterances, for reference only; do not split or translate them).\n\
         \n\
         Rules:\n\
         1. \"segments\": split the transcript into segments whose texts, joined, give the transcript exactly. \
         A segment that names a game term (a hero, item or ability in the glossary below) gets the term's id in \"ref\"; \
         every other segment has ref null. Recognize nicknames, abbreviations and misheard or mispronounced names, \
         and abilities called by their number, by \"ult\" (ability 4) or by part of their name, after their hero \
         (\"グレイブスの3番\" is Graves' ability 3, \"ドアマンのカート\" is Doorman's Luggage Cart). \
         Use the context to tell whose ability it is. Mark only what the streamer really means as a game term; \
         ordinary words stay plain (\"1番強い\", a web page, a shopping cart). A term's segment holds only the words \
         that name it (for \"グレイブスの3番\": \"グレイブス\" is the hero, \"の\" is plain, \"3番\" is the ability).\n\
         2. \"translations\": translate only the transcript into each requested language, naturally and casually, \
         like gamers talk. Write each game term as its id in brackets, like ⟦hero:13⟧, where the term belongs; \
         never write a term's name yourself. When a term's English name starts with \"The\", do not put an article right before it.\n\
         3. Do not attach inflectional endings to a term; rephrase instead (Russian: \"ульта ⟦hero:13⟧\", not \"⟦hero:13⟧а\"). \
         The only exception is the English possessive 's, which reads naturally: \"⟦hero:13⟧'s ⟦ability:1⟧\".\n\
         4. Keep common gamer words natural (ult, farm, push, gank).\n\
         \n\
         Glossary (id | kind | name | English name | owner of an ability):\n{}\n",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glossary() -> Glossary {
        serde_json::from_value(json!({
            "clientVersion": 1, "languages": ["ja", "en"],
            "entities": {
                "hero:13": { "kind": "hero", "className": "h", "names": { "ja": "ヘイズ", "en": "Haze" } },
                "item:1": { "kind": "item", "className": "i", "names": { "ja": "基礎拡張マガジン", "en": "Extended Magazine" } },
                "ability:8": { "kind": "ability", "className": "b", "names": { "ja": "バレットダンス", "en": "Bullet Dance" }, "hero": "hero:13", "abilitySlot": 4 },
                "ability:7": { "kind": "ability", "className": "c", "names": { "ja": "スリープダガー", "en": "Sleep Dagger" }, "hero": "hero:13", "abilitySlot": 1 }
            }
        }))
        .unwrap()
    }

    fn text(t: &str) -> Token {
        Token::Text {
            text: t.into(),
            ruby: None,
        }
    }

    fn ent(r: &str) -> Token {
        Token::Entity {
            reference: r.into(),
            spoken: None,
            ruby: None,
        }
    }

    fn entity(reference: &str, spoken: &str) -> Span {
        Span::Entity {
            reference: reference.into(),
            spoken: spoken.into(),
        }
    }

    fn analysis(segments: &[(&str, Option<&str>)], translations: &[(&str, &str)]) -> Analysis {
        Analysis {
            segments: segments
                .iter()
                .map(|(text, reference)| Segment {
                    text: (*text).into(),
                    reference: reference.map(Into::into),
                })
                .collect(),
            translations: translations
                .iter()
                .map(|(l, t)| ((*l).into(), (*t).into()))
                .collect(),
        }
    }

    #[test]
    fn spans_keep_known_terms_and_merge_the_rest() {
        let g = glossary();
        let a = analysis(
            &[
                ("ヘイズ", Some("hero:13")),
                ("の", None),
                ("ウルト", Some("ability:8")),
                ("と", None),
                ("謎", Some("item:999")),
                ("", Some("hero:13")),
                ("基礎", Some("item:1")),
                ("拡張マガジン", Some("item:1")),
            ],
            &[],
        );
        assert_eq!(
            a.spans("ヘイズのウルトと謎基礎拡張マガジン", Some(&g)),
            [
                entity("hero:13", "ヘイズ"),
                Span::Text("の".into()),
                entity("ability:8", "ウルト"),
                Span::Text("と謎".into()),
                entity("item:1", "基礎拡張マガジン"),
            ]
        );
    }

    #[test]
    fn spans_that_do_not_join_to_the_transcript_are_not_used() {
        let a = analysis(&[("ヘイズ", Some("hero:13")), ("強い", None)], &[]);
        assert_eq!(
            a.spans("ヘイズは強い", Some(&glossary())),
            [Span::Text("ヘイズは強い".into())]
        );
    }

    #[test]
    fn resolve_turns_ids_into_entities_and_drops_unknown_ones() {
        assert_eq!(
            resolve(
                "⟦hero:13⟧'s ⟦ ability:8 ⟧ beat ⟦item:1⟧ easily, ask ⟦hero:2⟧'s team ⟦oops",
                &["hero:13", "ability:8"]
            ),
            [
                ent("hero:13"),
                text("'s "),
                ent("ability:8"),
                text(" beat easily, ask team ⟦oops"),
            ]
        );
    }

    #[test]
    fn translations_drop_unrequested_languages() {
        let a = analysis(
            &[("ヘイズ", Some("hero:13")), ("勝ち", None)],
            &[("en", "⟦hero:13⟧ wins⟦ability:8⟧"), ("ru", "ignored")],
        );
        let t = a.translations(&["en".into()], Some(&glossary()));
        assert_eq!(t.len(), 1);
        // 原文で見つけていない用語（ability:8）は訳文にも出さない
        assert_eq!(t["en"], [ent("hero:13"), text(" wins")]);
    }

    fn enricher(targets: &[&str]) -> Enricher {
        Enricher::new(
            Access::key("key".into()),
            "model".into(),
            "model".into(),
            None,
            "ja".into(),
            targets.iter().map(|t| (*t).to_owned()).collect(),
            // 保存しない（persist を動かさない）ので、ファイルは作られない
            Arc::new(Meter::new("usage.json".into())),
        )
    }

    #[test]
    fn system_lists_terms_with_the_owner_of_abilities() {
        let system = system_prompt("ja", Some(&glossary()));
        assert!(system.contains("\nhero:13 | hero | ヘイズ | Haze\n"));
        assert!(system.contains(
            "\nability:8 | ability | バレットダンス | Bullet Dance | ability 4 of Haze (hero:13)\n"
        ));
        // 話す言語が英語なら、名前は 1 つ
        // 話す言語が英語でも列は揃える
        assert!(
            system_prompt("en", Some(&glossary())).contains("\nhero:13 | hero | Haze | Haze\n")
        );
    }

    #[test]
    fn system_is_rebuilt_when_glossary_version_changes() {
        let e = enricher(&["en"]);
        let mut g = glossary();
        let first = e.system(Some(&g));
        assert!(Arc::ptr_eq(&first, &e.system(Some(&g))));
        g.client_version = 2;
        let second = e.system(Some(&g));
        assert!(!Arc::ptr_eq(&first, &second));
        g.languages.push("ru".into());
        assert!(!Arc::ptr_eq(&second, &e.system(Some(&g))));
    }

    #[test]
    fn targets_exclude_source_and_unknown_languages() {
        assert_eq!(enricher(&["en", "ja", "xx"]).targets, ["en"]);
    }

    #[test]
    fn schema_asks_translations_only_with_targets() {
        let schema = enricher(&["en"]).schema();
        assert_eq!(schema["required"], json!(["segments", "translations"]));
        assert_eq!(
            schema["properties"]["translations"]["required"],
            json!(["en"])
        );
        assert_eq!(enricher(&[]).schema()["required"], json!(["segments"]));
    }

    #[tokio::test]
    async fn no_llm_call_without_glossary_targets_and_furigana() {
        // API キーもモデルも偽物なので、呼べばエラーになって warn が出るだけ。呼ばずに返ることを確かめる
        let e = enricher(&["ja"]);
        let started = std::time::Instant::now();
        let out = e.enrich("ヘイズ強い", &[], None, false).await;
        assert!(out.translations.is_empty());
        assert_eq!(out.source, [text("ヘイズ強い")]);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    fn seg(text: &str, ruby: Option<&str>) -> RubySegment {
        RubySegment {
            text: text.into(),
            ruby: ruby.map(Into::into),
        }
    }

    #[test]
    fn source_tokens_place_sentence_furigana_on_spans() {
        let spans = [entity("hero:13", "ヘイズ"), Span::Text("は強い".into())];
        let ruby = [
            seg("ヘイズは", None),
            seg("強", Some("つよ")),
            seg("い", None),
        ];
        assert_eq!(
            source_tokens(&spans, &ruby),
            [
                Token::Entity {
                    reference: "hero:13".into(),
                    spoken: Some("ヘイズ".into()),
                    ruby: None,
                },
                text("は"),
                Token::Text {
                    text: "強".into(),
                    ruby: Some("つよ".into())
                },
                text("い"),
            ]
        );
        // ふりがながなければ、区切りだけ
        assert_eq!(
            source_tokens(&spans, &[]),
            [
                Token::Entity {
                    reference: "hero:13".into(),
                    spoken: Some("ヘイズ".into()),
                    ruby: None,
                },
                text("は強い"),
            ]
        );
    }

    #[test]
    fn source_tokens_give_entities_their_own_furigana() {
        // 「ヘイズの一番スキル」。用語の中に収まる読みは用語に、区切りをまたぐ読みは付けない
        let spans = [
            Span::Text("見て".into()),
            Span::Entity {
                reference: "ability:7".into(),
                spoken: "一番スキル".into(),
            },
        ];
        let ruby = [
            seg("見", Some("み")),
            seg("て", None),
            seg("一番", Some("いちばん")),
            seg("スキル", None),
        ];
        assert_eq!(
            source_tokens(&spans, &ruby),
            [
                Token::Text {
                    text: "見".into(),
                    ruby: Some("み".into())
                },
                text("て"),
                Token::Entity {
                    reference: "ability:7".into(),
                    spoken: Some("一番スキル".into()),
                    ruby: Some(vec![seg("一番", Some("いちばん")), seg("スキル", None)]),
                },
            ]
        );
        let straddling = [seg("見て一", Some("みていち")), seg("番スキル", None)];
        assert_eq!(
            source_tokens(&spans, &straddling),
            [
                text("見て"),
                Token::Entity {
                    reference: "ability:7".into(),
                    spoken: Some("一番スキル".into()),
                    ruby: None,
                },
            ]
        );
    }

    #[test]
    fn without_a_glossary_the_prompt_is_not_about_the_game() {
        assert!(!system_prompt("ja", None).contains("Deadlock"));
        assert!(system_prompt("ja", Some(&glossary())).contains("Deadlock"));
    }
}
