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
    /// `⟦n⟧` → 種類（hero / item / ability）。英語名が冠詞から始まる（The Doorman）ならそう添える
    kinds: BTreeMap<String, String>,
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
                    let entity = glossary.and_then(|g| g.entities.get(reference));
                    let mut kind = entity
                        .map_or("game term", |e| kind_label(e.kind))
                        .to_owned();
                    // 冠詞ごとの名前の前に、LLM が冠詞を足さないように（「the The Doorman」にしない）
                    if entity
                        .and_then(|e| e.names.get("en"))
                        .is_some_and(|name| name.starts_with("The "))
                    {
                        kind.push_str(", its name already starts with \"The\"");
                    }
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
                    // 捨てた記号の所有格（⟦9⟧'s）も残さない
                    let after = &rest[end + '⟧'.len_utf8()..];
                    let after = ["'s", "’s"]
                        .iter()
                        .find_map(|s| after.strip_prefix(s))
                        .unwrap_or(after);
                    if buf.ends_with(' ') && after.starts_with(' ') {
                        buf.pop();
                    }
                    rest = after;
                    continue;
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
    /// 先頭の文字（`fold` で揃えたもの） → (揃えた語の文字列, 参照)。長い順
    by_first: HashMap<char, Vec<(Vec<char>, String)>>,
    /// (ヒーローの参照, 何番目のスキルか) → スキルの参照
    abilities: HashMap<(String, u8), String>,
    /// ヒーローの参照 → そのヒーローのスキルの名前と別名（小文字にしたもの）とスキルの参照（日本語だけ）
    hero_ability_names: HashMap<String, Vec<(Vec<char>, String)>>,
    heroes: HashSet<String>,
    lang: String,
}

impl Matcher {
    /// 1 文字の語は誤検出が多いので使わない
    const MIN_CHARS: usize = 2;

    fn new(glossary: &Glossary, lang: &str) -> Self {
        // 表記 → 参照。公式名が先に決まり、別名は公式名を上書きしない。
        // 複数のエンティティが同じ表記を持つときは、公式名なら先に見つけたもの、別名なら
        // どれとも決められないので使わない（None）
        let mut terms: HashMap<Vec<char>, Option<&String>> = HashMap::new();
        for (reference, entity) in &glossary.entities {
            if let Some(name) = entity.names.get(lang) {
                let found = *terms.entry(fold(name)).or_insert(Some(reference));
                // 書き方を揃えたら別の用語の公式名と同じになった（先に見つけたものを使う）
                if found != Some(reference) {
                    tracing::warn!("name {name} is spelled like another term's name");
                }
            }
        }
        let names: HashSet<Vec<char>> = terms.keys().cloned().collect();
        for (reference, entity) in &glossary.entities {
            for alias in entity.aliases.get(lang).into_iter().flatten() {
                let chars = fold(alias);
                if names.contains(&chars) {
                    continue;
                }
                terms
                    .entry(chars)
                    .and_modify(|r| {
                        if r.is_some_and(|r| r != reference) {
                            tracing::warn!("alias {alias} belongs to several terms; not used");
                            *r = None;
                        }
                    })
                    .or_insert(Some(reference));
            }
        }
        let mut by_first: HashMap<char, Vec<(Vec<char>, String)>> = HashMap::new();
        for (chars, reference) in terms {
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
        let mut hero_ability_names: HashMap<String, Vec<(Vec<char>, String)>> = HashMap::new();
        for (reference, e) in &glossary.entities {
            let (Some(hero), "ja") = (&e.hero, lang) else {
                continue;
            };
            let names = e
                .names
                .get(lang)
                .into_iter()
                .chain(e.aliases.get(lang).into_iter().flatten());
            for name in names {
                let entry = (name.chars().map(lower).collect(), reference.clone());
                let names = hero_ability_names.entry(hero.clone()).or_default();
                if !names.contains(&entry) {
                    names.push(entry);
                }
            }
        }
        let heroes = glossary
            .entities
            .iter()
            .filter(|(_, e)| e.kind == EntityKind::Hero)
            .map(|(reference, _)| reference.clone())
            .collect();
        Self {
            by_first,
            abilities,
            hero_ability_names,
            heroes,
            lang: lang.to_owned(),
        }
    }

    fn split(&self, text: &str) -> Vec<Span> {
        let original: Vec<char> = text.chars().collect();
        let chars = fold(text);
        let mut spans = Vec::new();
        let mut buf = String::new();
        let mut i = 0;
        while i < chars.len() {
            let found = self.by_first.get(&chars[i]).and_then(|terms| {
                terms.iter().find(|(term, reference)| {
                    Self::matches_at(&chars, i, term)
                        || self.hero_before_slot(text, &chars, i, term, reference)
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
    /// `chars` は `text` を `fold` したもの
    fn hero_before_slot(
        &self,
        text: &str,
        chars: &[char],
        i: usize,
        term: &[char],
        reference: &str,
    ) -> bool {
        if !self.heroes.contains(reference) {
            return false;
        }
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
                (Span::Text(text), Some(hero)) => slot_reference(text, &self.lang)
                    .and_then(|(prefix, phrase, slot)| {
                        let ability = self.abilities.get(&(hero.clone(), slot))?;
                        Some((text, prefix, phrase, ability.clone()))
                    })
                    .or_else(|| {
                        let (prefix, phrase, ability) = self.ability_part(text, hero)?;
                        Some((text, prefix, phrase, ability))
                    }),
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

    /// 「ドアマンのカート」: ヒーロー名と「の」のすぐ後ろのカタカナの語が、そのヒーローのスキル名の
    /// 頭か後ろの部分（2 文字以上。ぴったり同じでもよい）なら、そのスキル（キャリーカート）。
    /// 2 つ以上のスキルに当てはまれば決めない。返すのは (「の」の長さ, 語の長さ, スキルの参照)。
    /// 「の」がないと、カタカナどうしがつながってヒーロー名が見つからないので、ここには来ない
    fn ability_part(&self, text: &str, hero: &str) -> Option<(usize, usize, String)> {
        let chars: Vec<char> = text.chars().collect();
        if chars.first() != Some(&'の') {
            return None;
        }
        let len = chars[1..].iter().take_while(|c| is_katakana(**c)).count();
        if len < Self::MIN_CHARS {
            return None;
        }
        // 書き方の揺れは、比べる部分ごとに揃える（名前の途中で切ると、前の文字で揃い方が変わる）
        let folded = |c: &[char]| fold(&c.iter().collect::<String>());
        let word = folded(&chars[1..1 + len]);
        let mut found: Option<&String> = None;
        for (name, ability) in self.hero_ability_names.get(hero)? {
            let head = name.get(..len).map(folded);
            let tail = name.len().checked_sub(len).map(|i| folded(&name[i..]));
            if head.as_ref() == Some(&word) || tail.as_ref() == Some(&word) {
                if found.is_some_and(|f| f != ability) {
                    return None;
                }
                found = Some(ability);
            }
        }
        Some((1, len, found?.clone()))
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

/// 照合のために、書き方の揺れを揃える（大文字小文字、カタカナの伸ばし方と書き分け）。
/// 音声認識や別名の書き方が公式名と少し違っても同じ用語にする（ページ → ペイジ、リコシェット → リコチェット）。
/// 文字数は変えない（元の文字列の同じ位置を切り出す）。イ・ウは前の文字で決まるので、語の先頭の
/// イ・ウは文の中ではカタカナの後ろだと揃い方が変わるが、そこはカタカナの語の途中なので元から一致させない
fn fold(text: &str) -> Vec<char> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let prev = i.checked_sub(1).map(|j| chars[j]);
            let next = chars.get(i + 1).copied();
            match c {
                // エイ / エー、オウ / オー（ペイジ / ページ、ゴウスト / ゴースト）
                'イ' if prev.is_some_and(|p| "エケセテネヘメレゲゼデベペェ".contains(p)) => {
                    'ー'
                }
                'ウ' if prev.is_some_and(|p| "オコソトノホモヨロゴゾドボポォョ".contains(p)) => {
                    'ー'
                }
                // シェ / チェ（リコシェ / リコチェット）
                'シ' if next == Some('ェ') => 'チ',
                'ヂ' => 'ジ',
                'ヅ' => 'ズ',
                _ => lower(c),
            }
        })
        .collect()
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
         Never replace a marker with a name. When \"terms\" says a name already starts with \"The\", do not put an article right before it.\n\
         3. Do not attach inflectional endings to a marker; rephrase instead (Russian: \"ульта ⟦1⟧\", not \"⟦1⟧а\"). \
         The only exception is the English possessive 's, which reads naturally: \"⟦1⟧'s ⟦2⟧\".\n\
         4. Keep common gamer words natural (ult, farm, push, gank).\n"
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
            BTreeMap::from([
                ("⟦1⟧".into(), "hero".into()),
                ("⟦2⟧".into(), "ability".into())
            ])
        );
    }

    #[test]
    fn resolve_turns_markers_into_entities_and_drops_unknown_ones() {
        let g = glossary();
        let spans = Matcher::new(&g, "ja").split("ヘイズのバレットダンス");
        let terms = Terms::new(&spans, Some(&g));
        assert_eq!(
            terms.resolve("⟦1⟧'s ⟦2⟧ beat ⟦9⟧ easily, ask ⟦9⟧'s team ⟦oops"),
            [
                ent("hero:13"),
                text("'s "),
                ent("ability:8"),
                text(" beat easily, ask team ⟦oops"),
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
    fn part_of_an_ability_name_after_the_hero_means_that_ability() {
        let g: Glossary = serde_json::from_value(json!({
            "clientVersion": 1, "languages": ["ja"],
            "entities": {
                "hero:69": { "kind": "hero", "className": "d", "names": { "ja": "ドアマン" }, "aliases": {} },
                "ability:1": { "kind": "ability", "className": "a", "names": { "ja": "コールベル" }, "aliases": {},
                               "hero": "hero:69", "abilitySlot": 1 },
                "ability:3": { "kind": "ability", "className": "c", "names": { "ja": "キャリーカート" },
                               "aliases": { "ja": ["ラゲッジカート"] }, "hero": "hero:69", "abilitySlot": 3 },
                "ability:4": { "kind": "ability", "className": "e", "names": { "ja": "ホテルゲスト" }, "aliases": {},
                               "hero": "hero:69", "abilitySlot": 4 },
                "ability:9": { "kind": "ability", "className": "f", "names": { "ja": "カートレース" }, "aliases": {},
                               "hero": "hero:1", "abilitySlot": 1 },
            }
        }))
        .unwrap();
        let m = Matcher::new(&g, "ja");
        assert_eq!(
            m.split("ドアマンのカート強い"),
            [
                entity("hero:69", "ドアマン"),
                Span::Text("の".into()),
                entity("ability:3", "カート"),
                Span::Text("強い".into()),
            ]
        );
        assert_eq!(
            m.split("ドアマンのベル"),
            [
                entity("hero:69", "ドアマン"),
                Span::Text("の".into()),
                entity("ability:1", "ベル")
            ]
        );
        // ヒーロー名がなければ、ほかのヒーローのスキル（カートレース）とも取り違えない
        assert_eq!(m.split("カート強い"), [Span::Text("カート強い".into())]);
        // 別名の頭の部分でもよい
        assert_eq!(
            m.split("ドアマンのラゲッジ"),
            [
                entity("hero:69", "ドアマン"),
                Span::Text("の".into()),
                entity("ability:3", "ラゲッジ")
            ]
        );
        // 長いカタカナ語の一部（カートリッジ）や 1 文字では決めない
        for text in ["ドアマンのカートリッジ", "ドアマンのト"] {
            assert_eq!(
                m.split(text),
                [
                    entity("hero:69", "ドアマン"),
                    Span::Text(text["ドアマン".len()..].into())
                ],
                "{text}"
            );
        }
    }

    #[test]
    fn part_shared_by_two_abilities_is_not_decided() {
        let g: Glossary = serde_json::from_value(json!({
            "clientVersion": 1, "languages": ["ja"],
            "entities": {
                "hero:1": { "kind": "hero", "className": "h", "names": { "ja": "ヒーロー" }, "aliases": {} },
                "ability:1": { "kind": "ability", "className": "a", "names": { "ja": "ファイアボム" }, "aliases": {},
                               "hero": "hero:1", "abilitySlot": 1 },
                "ability:2": { "kind": "ability", "className": "b", "names": { "ja": "アイスボム" }, "aliases": {},
                               "hero": "hero:1", "abilitySlot": 2 },
            }
        }))
        .unwrap();
        assert_eq!(
            Matcher::new(&g, "ja").split("ヒーローのボム"),
            [entity("hero:1", "ヒーロー"), Span::Text("のボム".into())]
        );
    }

    #[test]
    fn spelling_variants_match_the_same_term() {
        let g: Glossary = serde_json::from_value(json!({
            "clientVersion": 1, "languages": ["ja"],
            "entities": {
                "hero:67": { "kind": "hero", "className": "p", "names": { "ja": "ペイジ" }, "aliases": {} },
                "item:1": { "kind": "item", "className": "r", "names": { "ja": "リコシェ" },
                            "aliases": { "ja": ["リコチェット"] } },
            }
        }))
        .unwrap();
        let m = Matcher::new(&g, "ja");
        // 音声認識が書くよく使う言葉の形でも、話した言葉はそのまま残す
        assert_eq!(
            m.split("ページ強い"),
            [entity("hero:67", "ページ"), Span::Text("強い".into())]
        );
        assert_eq!(
            m.split("リコシェット買った"),
            [
                entity("item:1", "リコシェット"),
                Span::Text("買った".into())
            ]
        );
        // 揃えても、カタカナの語の一部では一致させない
        assert_eq!(m.split("ページング"), [Span::Text("ページング".into())]);
    }

    #[test]
    fn aliases_that_fold_to_the_same_spelling_are_not_used() {
        let g: Glossary = serde_json::from_value(json!({
            "clientVersion": 1, "languages": ["ja"],
            "entities": {
                "item:1": { "kind": "item", "className": "a", "names": { "ja": "アイテムA" },
                            "aliases": { "ja": ["リコシェット"] } },
                "item:2": { "kind": "item", "className": "b", "names": { "ja": "アイテムB" },
                            "aliases": { "ja": ["リコチェット"] } },
            }
        }))
        .unwrap();
        // 書き方を揃えると同じになるので、どちらとも決められない
        assert_eq!(
            Matcher::new(&g, "ja").split("リコシェット"),
            [Span::Text("リコシェット".into())]
        );
    }

    #[test]
    fn terms_tell_when_the_english_name_has_an_article() {
        let mut g = glossary();
        g.entities
            .get_mut("hero:13")
            .unwrap()
            .names
            .insert("en".into(), "The Doorman".into());
        let terms = Terms::new(
            &[entity("hero:13", "ドアマン"), entity("ability:8", "ウルト")],
            Some(&g),
        );
        assert_eq!(
            terms.kinds["⟦1⟧"],
            "hero, its name already starts with \"The\""
        );
        assert_eq!(terms.kinds["⟦2⟧"], "ability");
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
