//! 確定した字幕に、固有名詞・ふりがな・翻訳を付ける（DESIGN.md 9 章）。
//!
//! - 固有名詞: 辞書の名前と略称を最長一致で探す（コードで決める）
//! - ふりがな: 形態素解析（`furigana`）で決める
//! - 翻訳: LLM。固有名詞は中身のない記号 `⟦1⟧` にして渡し、訳文の記号をエンティティに戻す。
//!   名前を見せないので、LLM が名前を訳して書くことがない（各言語の公式名は描画時に辞書から出す）
//!
//! LLM にトークン分けや読みまで任せると、区切りやルビがずれるので任せない。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::furigana::Furigana;
use crate::lang;
use crate::types::{Enriched, EntityKind, Glossary, LangCode, Token};

const CHAT_URL: &str = "https://api.mistral.ai/v1/chat/completions";
const TIMEOUT: Duration = Duration::from_secs(8);

pub struct Enricher {
    http: reqwest::Client,
    api_key: String,
    model: String,
    source_lang: LangCode,
    targets: Vec<LangCode>,
    /// 固定の先頭部分（プロンプトキャッシュが効く）
    system: String,
    /// 日本語のときだけ
    furigana: Option<Furigana>,
    /// 辞書の版ごとに作り直す（辞書が後から届いたり、作り直されたりする）
    matcher: Mutex<Option<(u32, Arc<Matcher>)>>,
}

impl Enricher {
    pub fn new(
        api_key: String,
        model: String,
        source_lang: LangCode,
        targets: Vec<LangCode>,
    ) -> anyhow::Result<Self> {
        let targets = lang::normalize(targets.into_iter().filter(|t| *t != source_lang));
        let furigana = (source_lang == "ja").then(Furigana::new).transpose()?;
        Ok(Self {
            http: reqwest::Client::new(),
            api_key,
            model,
            system: system_prompt(&source_lang),
            source_lang,
            targets,
            furigana,
            matcher: Mutex::new(None),
        })
    }

    /// `context` は直前の確定字幕（古い順）。翻訳に失敗しても、原文の固有名詞とふりがなは返す。
    /// 雑音（咳やフィラーだけ）はバッチの文字起こしが空になるので、ここでは判定しない
    pub async fn enrich(
        &self,
        text: &str,
        context: &[String],
        glossary: Option<&Glossary>,
    ) -> Enriched {
        let spans = match glossary {
            Some(g) => self.matcher(g).split(text),
            None => vec![Span::Text(text.to_owned())],
        };
        let source = self.source_tokens(&spans);
        let translations = if self.targets.is_empty() {
            BTreeMap::new()
        } else {
            let terms = Terms::new(&spans, glossary);
            match self.translate(&terms, context).await {
                Ok(raw) => self.translations(raw, &terms),
                Err(e) => {
                    tracing::warn!("translate: {e:#}");
                    BTreeMap::new()
                }
            }
        };
        Enriched {
            source,
            translations,
        }
    }

    fn matcher(&self, glossary: &Glossary) -> Arc<Matcher> {
        let mut cached = self.matcher.lock().unwrap();
        match &*cached {
            Some((version, m)) if *version == glossary.client_version => m.clone(),
            _ => {
                let m = Arc::new(Matcher::new(glossary, &self.source_lang));
                *cached = Some((glossary.client_version, m.clone()));
                m
            }
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
        terms: &Terms,
        context: &[String],
    ) -> anyhow::Result<BTreeMap<LangCode, String>> {
        let user = json!({
            "context": context,
            "transcript": terms.transcript,
            "terms": terms.kinds,
        });
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": self.system },
                { "role": "user", "content": user.to_string() },
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

    /// 頼んでいない言語は捨て、訳文の `⟦n⟧` をエンティティに戻す
    fn translations(
        &self,
        raw: BTreeMap<LangCode, String>,
        terms: &Terms,
    ) -> BTreeMap<LangCode, Vec<Token>> {
        raw.into_iter()
            .filter(|(lang, _)| self.targets.contains(lang))
            .map(|(lang, text)| (lang, terms.resolve(&text)))
            .collect()
    }
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

/// LLM に渡す原文。固有名詞は出てきた順に `⟦1⟧`, `⟦2⟧` … にする（同じものは同じ番号）
struct Terms {
    transcript: String,
    /// `⟦n⟧` → 種類（hero / item / ability）
    kinds: BTreeMap<String, &'static str>,
    refs: Vec<String>,
}

impl Terms {
    fn new(spans: &[Span], glossary: Option<&Glossary>) -> Self {
        let mut refs: Vec<String> = Vec::new();
        let mut kinds = BTreeMap::new();
        let mut transcript = String::new();
        for span in spans {
            match span {
                Span::Text(t) => transcript.push_str(t),
                Span::Entity { reference, .. } => {
                    let n = match refs.iter().position(|r| r == reference) {
                        Some(i) => i + 1,
                        None => {
                            refs.push(reference.clone());
                            refs.len()
                        }
                    };
                    let marker = marker(n);
                    let kind = glossary
                        .and_then(|g| g.entities.get(reference))
                        .map_or("game term", |e| kind_label(e.kind));
                    transcript.push_str(&marker);
                    kinds.insert(marker, kind);
                }
            }
        }
        Self {
            transcript,
            kinds,
            refs,
        }
    }

    /// 訳文の `⟦n⟧` をエンティティにする。知らない番号は捨て、そのために空いた空白を詰める
    fn resolve(&self, text: &str) -> Vec<Token> {
        let mut tokens: Vec<Token> = Vec::new();
        let mut buf = String::new();
        let mut rest = text;
        while let Some(start) = rest.find('⟦') {
            let Some(len) = rest[start..].find('⟧') else {
                break;
            };
            let end = start + len;
            buf.push_str(&rest[..start]);
            let n: Option<usize> = rest[start + '⟦'.len_utf8()..end].trim().parse().ok();
            match n.and_then(|n| self.refs.get(n.wrapping_sub(1))) {
                Some(reference) => {
                    push_text(&mut tokens, std::mem::take(&mut buf));
                    tokens.push(Token::Entity {
                        reference: reference.clone(),
                        spoken: None,
                    });
                }
                None => {
                    if buf.ends_with(' ') && rest[end + '⟧'.len_utf8()..].starts_with(' ') {
                        buf.pop();
                    }
                }
            }
            rest = &rest[end + '⟧'.len_utf8()..];
        }
        buf.push_str(rest);
        push_text(&mut tokens, buf);
        tokens
    }
}

fn marker(n: usize) -> String {
    format!("⟦{n}⟧")
}

fn push_text(tokens: &mut Vec<Token>, text: String) {
    if !text.is_empty() {
        tokens.push(Token::Text { text, ruby: None });
    }
}

fn kind_label(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Hero => "hero",
        EntityKind::Item => "item",
        EntityKind::Ability => "ability",
    }
}

/// 辞書の名前と略称を最長一致で探す
#[derive(Default)]
struct Matcher {
    /// 先頭の文字（小文字にしたもの） → (語の文字列, 参照)。長い順
    by_first: HashMap<char, Vec<(Vec<char>, String)>>,
}

impl Matcher {
    /// 1 文字の語は誤検出が多いので使わない
    const MIN_CHARS: usize = 2;

    fn new(glossary: &Glossary, lang: &str) -> Self {
        let mut seen: HashSet<Vec<char>> = HashSet::new();
        let mut by_first: HashMap<char, Vec<(Vec<char>, String)>> = HashMap::new();
        for (reference, entity) in &glossary.entities {
            let names = entity.names.get(lang).into_iter();
            let aliases = entity.aliases.get(lang).into_iter().flatten();
            for term in names.chain(aliases) {
                let chars: Vec<char> = term.chars().map(lower).collect();
                // 同じ表記を複数のエンティティが持つときは、先に見つけたものにする
                if chars.len() >= Self::MIN_CHARS && seen.insert(chars.clone()) {
                    by_first
                        .entry(chars[0])
                        .or_default()
                        .push((chars, reference.clone()));
                }
            }
        }
        for terms in by_first.values_mut() {
            terms.sort_by_key(|(t, _)| std::cmp::Reverse(t.len()));
        }
        Self { by_first }
    }

    fn split(&self, text: &str) -> Vec<Span> {
        let original: Vec<char> = text.chars().collect();
        let chars: Vec<char> = original.iter().copied().map(lower).collect();
        let mut spans = Vec::new();
        let mut buf = String::new();
        let mut i = 0;
        while i < chars.len() {
            let found = self.by_first.get(&chars[i]).and_then(|terms| {
                terms
                    .iter()
                    .find(|(term, _)| Self::matches_at(&chars, i, term))
            });
            match found {
                Some((term, reference)) => {
                    if !buf.is_empty() {
                        spans.push(Span::Text(std::mem::take(&mut buf)));
                    }
                    spans.push(Span::Entity {
                        reference: reference.clone(),
                        spoken: original[i..i + term.len()].iter().collect(),
                    });
                    i += term.len();
                }
                None => {
                    buf.push(original[i]);
                    i += 1;
                }
            }
        }
        if !buf.is_empty() {
            spans.push(Span::Text(buf));
        }
        spans
    }

    /// 別の語の一部なら一致としない。
    /// カタカナの語は前後がカタカナでないこと（「セブンイレブン」の「セブン」）、
    /// 空白で区切る文字の語は前後が文字や数字でないこと（「Hazel」の「Haze」）
    fn matches_at(chars: &[char], i: usize, term: &[char]) -> bool {
        if !chars[i..].starts_with(term) {
            return false;
        }
        let before = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
        let after = chars.get(i + term.len()).copied();
        let joins = |edge: char, neighbor: Option<char>| {
            neighbor.is_some_and(|n| {
                (is_katakana(edge) && is_katakana(n))
                    || (is_spaced_word_char(edge) && is_spaced_word_char(n))
            })
        };
        !joins(term[0], before) && !joins(term[term.len() - 1], after)
    }
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn is_katakana(c: char) -> bool {
    matches!(c, '\u{30A0}'..='\u{30FF}')
}

/// 単語を空白で区切る文字体系の文字（ラテン・キリル・ギリシャ文字と数字）
fn is_spaced_word_char(c: char) -> bool {
    c.is_alphanumeric()
        && !matches!(c,
            '\u{3040}'..='\u{30FF}' // かな
            | '\u{3400}'..='\u{9FFF}' // 漢字
            | '\u{AC00}'..='\u{D7AF}' // ハングル
            | '\u{0E00}'..='\u{0E7F}' // タイ文字
        )
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

fn system_prompt(source_lang: &str) -> String {
    let lang_label = lang::find(source_lang).map_or(source_lang, |l| l.label);
    format!(
        "You translate live commentary from a streamer playing the video game Deadlock. \
         The streamer speaks {lang_label}. Each request has \"transcript\" (one utterance from speech recognition), \
         \"terms\" and \"context\" (the previous utterances, for reference only; do not translate them).\n\
         In the transcript, game terms are replaced with markers like ⟦1⟧. \"terms\" tells what each marker is (hero, item or ability).\n\
         \n\
         Rules:\n\
         1. \"translations\": translate only the transcript into each requested language, naturally and casually, like gamers talk.\n\
         2. Copy every marker such as ⟦1⟧ unchanged into each translation, where that term belongs. \
         Never replace a marker with a name, and do not attach possessive endings to it (write \"the ult of ⟦1⟧\", not \"⟦1⟧'s ult\").\n\
         3. Keep common gamer words natural (ult, farm, push, gank).\n"
    )
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
                "ability:9": { "kind": "ability", "className": "a", "names": { "ja": "ダンス", "en": "Dash" }, "aliases": {} },
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
    fn matcher_respects_word_boundaries_and_case_for_spaced_languages() {
        let m = Matcher::new(&glossary(), "en");
        assert_eq!(
            m.split("Hazel opened the dashboard, haze used Dash."),
            [
                Span::Text("Hazel opened the dashboard, ".into()),
                entity("hero:13", "haze"),
                Span::Text(" used ".into()),
                entity("ability:9", "Dash"),
                Span::Text(".".into()),
            ]
        );
    }

    #[test]
    fn terms_hide_names_behind_numbered_markers() {
        let g = glossary();
        let spans = Matcher::new(&g, "ja").split("ヘイズのバレットダンスとヘイズ");
        let terms = Terms::new(&spans, Some(&g));
        assert_eq!(terms.transcript, "⟦1⟧の⟦2⟧と⟦1⟧");
        assert_eq!(
            terms.kinds,
            BTreeMap::from([("⟦1⟧".into(), "hero"), ("⟦2⟧".into(), "ability")])
        );
    }

    #[test]
    fn resolve_turns_markers_into_entities_and_drops_unknown_ones() {
        let g = glossary();
        let spans = Matcher::new(&g, "ja").split("ヘイズのバレットダンス");
        let terms = Terms::new(&spans, Some(&g));
        assert_eq!(
            terms.resolve("The ⟦2⟧ of ⟦1⟧ beat ⟦9⟧ easily ⟦oops"),
            [
                text("The "),
                ent("ability:8"),
                text(" of "),
                ent("hero:13"),
                text(" beat easily ⟦oops"),
            ]
        );
    }

    fn enricher(targets: &[&str]) -> Enricher {
        Enricher::new(
            "key".into(),
            "model".into(),
            "ja".into(),
            targets.iter().map(|t| (*t).to_owned()).collect(),
        )
        .unwrap()
    }

    #[test]
    fn source_tokens_combine_entities_and_furigana() {
        let e = enricher(&["en"]);
        let tokens = e.source_tokens(&e.matcher(&glossary()).split("ヘイズは強い"));
        assert_eq!(
            tokens,
            [
                Token::Entity {
                    reference: "hero:13".into(),
                    spoken: Some("ヘイズ".into())
                },
                text("は"),
                Token::Text {
                    text: "強".into(),
                    ruby: Some("つよ".into())
                },
                text("い"),
            ]
        );
    }

    #[test]
    fn matcher_is_rebuilt_when_glossary_version_changes() {
        let e = enricher(&["en"]);
        let mut g = glossary();
        let first = e.matcher(&g);
        assert!(Arc::ptr_eq(&first, &e.matcher(&g)));
        g.client_version = 2;
        assert!(!Arc::ptr_eq(&first, &e.matcher(&g)));
    }

    #[test]
    fn targets_exclude_source_and_unknown_languages() {
        assert_eq!(enricher(&["en", "ja", "xx"]).targets, ["en"]);
    }

    #[tokio::test]
    async fn no_targets_means_no_llm_call() {
        // API キーもモデルも偽物なので、呼べばエラーになって warn が出るだけ。呼ばずに返ることを確かめる
        let e = enricher(&["ja"]);
        let started = std::time::Instant::now();
        let out = e.enrich("ヘイズ強い", &[], Some(&glossary())).await;
        assert!(out.translations.is_empty());
        assert_eq!(out.source.len(), 3);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn translations_drop_unrequested_languages() {
        let e = enricher(&["en"]);
        let terms = Terms::new(&[entity("hero:13", "ヘイズ")], Some(&glossary()));
        let raw = BTreeMap::from([
            ("en".to_owned(), "⟦1⟧ wins".to_owned()),
            ("ru".to_owned(), "ignored".to_owned()),
        ]);
        let t = e.translations(raw, &terms);
        assert_eq!(t.len(), 1);
        assert_eq!(t["en"], [ent("hero:13"), text(" wins")]);
    }

    #[test]
    fn message_text_handles_thinking_parts() {
        let content = json!([{ "type": "thinking", "thinking": [] }, { "type": "text", "text": "{\"a\":1}" }]);
        assert_eq!(message_text(&content).as_deref(), Some("{\"a\":1}"));
    }

    #[test]
    fn schema_requires_each_target_language() {
        let schema = enricher(&["en"]).schema();
        assert_eq!(
            schema["properties"]["translations"]["required"],
            json!(["en"])
        );
    }
}
