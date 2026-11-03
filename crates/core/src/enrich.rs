//! 確定した字幕に、固有名詞・ふりがな・翻訳を付ける（DESIGN.md 9 章）。
//!
//! - 固有名詞: 辞書の名前と略称を最長一致で探す（コードで決める）
//! - ふりがな: LLM に文ごとに聞き（`furigana`）、翻訳と同時に問い合わせる。答えは確かめてから使う
//! - 翻訳: LLM。固有名詞は中身のない記号 `⟦1⟧` にして渡し、訳文の記号をエンティティに戻す。
//!   名前を見せないので、LLM が名前を訳して書くことがない（各言語の公式名は描画時に辞書から出す）
//!
//! 固有名詞の区切りは LLM に任せない。ふりがなは文字の位置で区切りに当てはめる（`source_tokens`）。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::furigana;
use crate::lang;
use crate::llm;
use crate::types::{Enriched, EntityKind, Glossary, LangCode, RubySegment, Token};
use crate::usage::{Meter, Use};

const TIMEOUT: Duration = Duration::from_secs(8);

pub struct Enricher {
    http: reqwest::Client,
    /// OpenAI の API キー
    api_key: String,
    model: String,
    /// ふりがなのモデル（辞書の名前の読みと同じ。DESIGN.md 8.2）
    readings_model: String,
    reasoning_effort: Option<String>,
    source_lang: LangCode,
    targets: Vec<LangCode>,
    /// 固定の先頭部分（プロンプトキャッシュが効く）
    system: String,
    meter: Arc<Meter>,
    /// 辞書の版ごとに作り直す（辞書が後から届いたり、作り直されたりする）
    matcher: Mutex<Option<(u32, Arc<Matcher>)>>,
}

impl Enricher {
    pub fn new(
        api_key: String,
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
            api_key,
            model,
            readings_model,
            reasoning_effort,
            system: system_prompt(&source_lang),
            source_lang,
            targets,
            matcher: Mutex::new(None),
            meter,
        }
    }

    /// `context` は直前の確定字幕（古い順）。翻訳に失敗しても、原文の固有名詞とふりがなは返す。
    /// 雑音（咳やフィラーだけ）はバッチの文字起こしが空になるので、ここでは判定しない
    pub async fn enrich(
        &self,
        text: &str,
        context: &[String],
        glossary: Option<&Glossary>,
        furigana: bool,
    ) -> Enriched {
        let spans = match glossary {
            Some(g) => self.matcher(g).split(text),
            None => vec![Span::Text(text.to_owned())],
        };
        let terms = Terms::new(&spans, glossary);
        let (ruby, translations) = tokio::join!(self.furigana(text, context, furigana), async {
            if self.targets.is_empty() {
                return BTreeMap::new();
            }
            match self.translate(&self.api_key, &terms, context).await {
                Ok(raw) => self.translations(raw, &terms),
                Err(e) => {
                    tracing::warn!("translate: {e:#}");
                    BTreeMap::new()
                }
            }
        });
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
            api_key: &self.api_key,
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

    async fn translate(
        &self,
        api_key: &str,
        terms: &Terms,
        context: &[String],
    ) -> anyhow::Result<BTreeMap<LangCode, String>> {
        let user = json!({
            "context": context,
            "transcript": terms.transcript,
            "terms": terms.kinds,
        });
        let out: Output = llm::chat_json(
            &self.http,
            llm::Request {
                api_key,
                model: &self.model,
                reasoning_effort: self.reasoning_effort.as_deref(),
                system: &self.system,
                user: user.to_string(),
                schema_name: "translation",
                schema: self.schema(),
                timeout: TIMEOUT,
                meter: &self.meter,
                purpose: Use::Translate,
            },
        )
        .await?;
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

/// ヒーロー名の直後の文字列が、スキルを番号で呼んでいるか。
/// (つなぎの文字数, 番号の表現の文字数, 何番目か) を返す（文字数は `char` の数）。
///
/// - 日本語: 「の」のあとに「ウルト」「アルティメット」、または「1スキ」「1番スキル」「2番目のスキル」など。
///   「1番強い」「一番好き」と区別できないので、番号は「スキル」「スキ」まで言ったときだけ
/// - 英語: 「's ult」「 ultimate」
fn slot_reference(text: &str, lang: &str) -> Option<(usize, usize, u8)> {
    let chars: Vec<char> = text.chars().map(lower).collect();
    let starts = |at: usize, word: &str| {
        let w: Vec<char> = word.chars().collect();
        chars[at..].starts_with(&w).then_some(w.len())
    };
    if lang == "ja" {
        let prefix = starts(0, "の").unwrap_or(0);
        for ult in ["アルティメット", "ウルティメット", "ウルト"] {
            if let Some(n) = starts(prefix, ult) {
                // 「ウルトラ」のように長いカタカナ語の一部なら違う
                let next = chars.get(prefix + n).copied();
                if !next.is_some_and(is_katakana) {
                    return Some((prefix, n, 4));
                }
            }
        }
        let c = *chars.get(prefix)?;
        let slot = match c.to_digit(10) {
            Some(d) => d,
            None => match ('１'..='４').position(|x| x == c) {
                Some(d) => d as u32 + 1,
                None => ['一', '二', '三', '四'].iter().position(|&x| x == c)? as u32 + 1,
            },
        };
        if !(1..=4).contains(&slot) {
            return None;
        }
        let slot = slot as u8;
        let mut at = prefix + 1;
        let ban = starts(at, "番目").or_else(|| starts(at, "番"));
        at += ban.unwrap_or(0);
        at += starts(at, "の").unwrap_or(0);
        // 「1番強い」「一番好き」と区別できないので「スキル」「スキ」まで言ったときだけ。
        // 「1スキン」「スキップ」のように長いカタカナ語の一部なら違う
        let skill = starts(at, "スキル").or_else(|| starts(at, "スキ"))?;
        at += skill;
        if chars.get(at).copied().is_some_and(is_katakana) {
            return None;
        }
        return Some((prefix, at - prefix, slot));
    }
    if lang == "en" {
        let prefix = starts(0, "'s ")
            .or_else(|| starts(0, "’s "))
            .or_else(|| starts(0, " "))?;
        for ult in ["ultimate", "ult"] {
            if let Some(n) = starts(prefix, ult)
                && !chars
                    .get(prefix + n)
                    .copied()
                    .is_some_and(is_spaced_word_char)
            {
                return Some((prefix, n, 4));
            }
        }
    }
    None
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
                        ruby: None,
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
    /// (ヒーローの参照, 何番目のスキルか) → スキルの参照
    abilities: HashMap<(String, u8), String>,
    heroes: HashSet<String>,
    lang: String,
}

impl Matcher {
    /// 1 文字の語は誤検出が多いので使わない
    const MIN_CHARS: usize = 2;

    fn new(glossary: &Glossary, lang: &str) -> Self {
        let key = |term: &str| -> Vec<char> { term.chars().map(lower).collect() };
        // 表記 → 参照。公式名が先に決まり、別名は公式名を上書きしない。
        // 複数のエンティティが同じ表記を持つときは、公式名なら先に見つけたもの、別名なら
        // どれとも決められないので使わない（None）
        let mut terms: HashMap<Vec<char>, Option<&String>> = HashMap::new();
        for (reference, entity) in &glossary.entities {
            if let Some(name) = entity.names.get(lang) {
                terms.entry(key(name)).or_insert(Some(reference));
            }
        }
        let names: HashSet<Vec<char>> = terms.keys().cloned().collect();
        for (reference, entity) in &glossary.entities {
            for alias in entity.aliases.get(lang).into_iter().flatten() {
                let chars = key(alias);
                if names.contains(&chars) {
                    continue;
                }
                terms
                    .entry(chars)
                    .and_modify(|r| {
                        if *r != Some(reference) {
                            *r = None;
                        }
                    })
                    .or_insert(Some(reference));
            }
        }
        let mut by_first: HashMap<char, Vec<(Vec<char>, String)>> = HashMap::new();
        for (chars, reference) in terms {
            if reference.is_none() {
                let alias: String = chars.iter().collect();
                tracing::warn!("alias {alias} belongs to several terms; not used");
            }
            if let Some(reference) = reference
                && chars.len() >= Self::MIN_CHARS
            {
                by_first
                    .entry(chars[0])
                    .or_default()
                    .push((chars, reference.clone()));
            }
        }
        for terms in by_first.values_mut() {
            terms.sort_by_key(|(t, _)| std::cmp::Reverse(t.len()));
        }
        let abilities = glossary
            .entities
            .iter()
            .filter_map(|(reference, e)| {
                Some(((e.hero.clone()?, e.ability_slot?), reference.clone()))
            })
            .collect();
        let heroes = glossary
            .entities
            .iter()
            .filter(|(_, e)| e.kind == EntityKind::Hero)
            .map(|(reference, _)| reference.clone())
            .collect();
        Self {
            by_first,
            abilities,
            heroes,
            lang: lang.to_owned(),
        }
    }

    fn split(&self, text: &str) -> Vec<Span> {
        let original: Vec<char> = text.chars().collect();
        let chars: Vec<char> = original.iter().copied().map(lower).collect();
        let mut spans = Vec::new();
        let mut buf = String::new();
        let mut i = 0;
        while i < chars.len() {
            let found = self.by_first.get(&chars[i]).and_then(|terms| {
                terms.iter().find(|(term, reference)| {
                    Self::matches_at(&chars, i, term)
                        || self.hero_before_slot(text, i, term, reference)
                })
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
        self.resolve_ability_slots(spans)
    }

    /// 「ヘイズウルト」のように「の」なしでスキルの番号が続くと、カタカナどうしがつながって
    /// ヒーロー名と見なせない（`matches_at`）ので、続きがスキルの番号なら認める
    fn hero_before_slot(&self, text: &str, i: usize, term: &[char], reference: &str) -> bool {
        if !self.heroes.contains(reference) {
            return false;
        }
        let chars: Vec<char> = text.chars().map(lower).collect();
        let joined_before = i
            .checked_sub(1)
            .is_some_and(|j| is_katakana(chars[j]) && is_katakana(term[0]));
        if !chars[i..].starts_with(term) || joined_before {
            return false;
        }
        let rest: String = text.chars().skip(i + term.len()).collect();
        slot_reference(&rest, &self.lang).is_some_and(|(prefix, ..)| prefix == 0)
    }

    /// 「ヘイズのウルト」「ヘイズの1番スキル」のように、ヒーロー名のすぐ後ろでスキルを番号で
    /// 呼んだら、そのヒーローのそのスキルにする（表示は話した言葉のまま）
    fn resolve_ability_slots(&self, spans: Vec<Span>) -> Vec<Span> {
        let mut out: Vec<Span> = Vec::with_capacity(spans.len());
        for span in spans {
            let hero = match out.last() {
                Some(Span::Entity { reference, .. }) if self.heroes.contains(reference) => {
                    Some(reference.clone())
                }
                _ => None,
            };
            let found = match (&span, &hero) {
                (Span::Text(text), Some(hero)) => {
                    slot_reference(text, &self.lang).and_then(|(prefix, phrase, slot)| {
                        let ability = self.abilities.get(&(hero.clone(), slot))?;
                        Some((text, prefix, phrase, ability.clone()))
                    })
                }
                _ => None,
            };
            let Some((text, prefix, phrase, ability)) = found else {
                out.push(span);
                continue;
            };
            let chars: Vec<char> = text.chars().collect();
            let piece = |a: usize, b: usize| chars[a..b].iter().collect::<String>();
            if prefix > 0 {
                out.push(Span::Text(piece(0, prefix)));
            }
            out.push(Span::Entity {
                reference: ability,
                spoken: piece(prefix, prefix + phrase),
            });
            if prefix + phrase < chars.len() {
                out.push(Span::Text(piece(prefix + phrase, chars.len())));
            }
        }
        out
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
                "ability:8": { "kind": "ability", "className": "b", "names": { "ja": "バレットダンス", "en": "Bullet Dance" }, "aliases": {}, "hero": "hero:13", "abilitySlot": 4 },
                "ability:7": { "kind": "ability", "className": "c", "names": { "ja": "スリープダガー", "en": "Sleep Dagger" }, "aliases": {}, "hero": "hero:13", "abilitySlot": 1 }
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
    fn hero_followed_by_an_ability_number_means_that_ability() {
        let m = Matcher::new(&glossary(), "ja");
        assert_eq!(
            m.split("ヘイズのウルトが強い"),
            vec![
                entity("hero:13", "ヘイズ"),
                Span::Text("の".into()),
                entity("ability:8", "ウルト"),
                Span::Text("が強い".into()),
            ]
        );
        for (text, spoken) in [
            ("ヘイズの1番スキル", "1番スキル"),
            ("ヘイズの一番スキル", "一番スキル"),
            ("ヘイズの1スキ", "1スキ"),
            ("ヘイズ１番目のスキル", "１番目のスキル"),
        ] {
            assert!(
                m.split(text).contains(&entity("ability:7", spoken)),
                "{text}"
            );
        }
        // 「の」なし
        assert_eq!(
            m.split("ヘイズウルト溜まった"),
            vec![
                entity("hero:13", "ヘイズ"),
                entity("ability:8", "ウルト"),
                Span::Text("溜まった".into()),
            ]
        );
        // 「一番好き」「1番強い」「1スキン」「ウルトラ」、持っていない番号は違う
        for text in [
            "ヘイズの一番好きなところ",
            "ヘイズの1番強い",
            "ヘイズの1スキン",
            "ヘイズのウルトラ",
            "ヘイズの3スキ",
            "ウルトが強い",
            "ヘイズウルトラマン",
        ] {
            assert!(
                !m.split(text).iter().any(|s| matches!(
                    s,
                    Span::Entity { reference, .. } if reference.starts_with("ability:")
                )),
                "{text}"
            );
        }

        let m = Matcher::new(&glossary(), "en");
        assert!(
            m.split("Haze's ult is up")
                .contains(&entity("ability:8", "ult"))
        );
        assert!(!m.split("Haze's ultra").iter().any(|s| matches!(
            s,
            Span::Entity { reference, .. } if reference.starts_with("ability:")
        )));
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
    fn matcher_prefers_official_names_and_drops_shared_aliases() {
        let g: Glossary = serde_json::from_value(serde_json::json!({
            "clientVersion": 1, "languages": ["ja"],
            "entities": {
                "item:1": { "kind": "item", "className": "a", "names": { "ja": "基礎拡張マガジン" },
                            "aliases": { "ja": ["カクマガ", "マガジン", "ヘイズ"] } },
                "item:2": { "kind": "item", "className": "b", "names": { "ja": "巨大拡張マガジン" },
                            "aliases": { "ja": ["マガジン"] } },
                "hero:13": { "kind": "hero", "className": "h", "names": { "ja": "ヘイズ" }, "aliases": {} }
            }
        }))
        .unwrap();
        let spans = Matcher::new(&g, "ja").split("ヘイズのカクマガとマガジン");
        assert_eq!(
            spans,
            vec![
                entity("hero:13", "ヘイズ"),
                Span::Text("の".into()),
                entity("item:1", "カクマガ"),
                Span::Text("とマガジン".into()),
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
            "model".into(),
            None,
            "ja".into(),
            targets.iter().map(|t| (*t).to_owned()).collect(),
            // 保存しない（persist を動かさない）ので、ファイルは作られない
            Arc::new(Meter::new("usage.json".into())),
        )
    }

    fn seg(text: &str, ruby: Option<&str>) -> RubySegment {
        RubySegment {
            text: text.into(),
            ruby: ruby.map(Into::into),
        }
    }

    #[test]
    fn source_tokens_place_sentence_furigana_on_spans() {
        let e = enricher(&["en"]);
        let spans = e.matcher(&glossary()).split("ヘイズは強い");
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
    async fn no_llm_call_without_targets_and_furigana() {
        // API キーもモデルも偽物なので、呼べばエラーになって warn が出るだけ。呼ばずに返ることを確かめる
        // （翻訳先がなく、漢字もないので、翻訳もふりがなも聞かない）
        let e = enricher(&["ja"]);
        let started = std::time::Instant::now();
        let out = e.enrich("ヘイズつよい", &[], Some(&glossary()), true).await;
        assert!(out.translations.is_empty());
        assert_eq!(out.source.len(), 2);
        // ふりがなを出さない設定なら、漢字があっても聞かない
        let out = e.enrich("ヘイズ強い", &[], Some(&glossary()), false).await;
        assert_eq!(out.source.len(), 2);
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
    fn schema_requires_each_target_language() {
        let schema = enricher(&["en"]).schema();
        assert_eq!(
            schema["properties"]["translations"]["required"],
            json!(["en"])
        );
    }
}
