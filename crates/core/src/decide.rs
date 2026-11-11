//! 用語の判定を、PC の中の候補出しと、判定モデルの確かめに分ける（DESIGN.md 9.7）。
//!
//! 判定モデル（OpenAI の Decisions API、TypeSafe AI の Jev、OpenRouter の判定モデル）は文を書かずに選択肢から選ぶだけなので、LLM に辞書の全部を渡して文を区切らせるより
//! ずっと速く安い。ただ文の中から用語を探すことはできないので、候補は PC の中で出す:
//! 辞書の名前と別名の一致、名前の似たカタカナの言葉（略称・名前の一部・聞き違い）、「ウルト」と番号。
//! 候補は多めに出し、本当にその用語か（ふつうの言葉ではないか、どのスキルか）を文ごとに選ばせる

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::access::Access;
use crate::assets::Loaded;
use crate::enrich::Span;
use crate::secrets::Provider;
use crate::share;
use crate::types::{EntityKind, Glossary, LangCode};
use crate::usage::{Meter, Tokens, Use};

/// 応答は 0.3 秒ほど。待たせすぎずに、辞書の一致に切り替える
const TIMEOUT: Duration = Duration::from_secs(3);

/// 1 つの言葉に当てる、似た名前の用語の数
const MAX_SIMILAR: usize = 5;

pub struct Decider {
    access: Access,
    /// OpenAI の Decisions API が使えるのは `gpt-6-luna` だけ
    model: &'static str,
    source_lang: LangCode,
    meter: Arc<Meter>,
    /// 辞書と別名から作った索引（辞書が差し替わったら作り直す）
    index: Mutex<Option<(Loaded, Arc<Index>)>>,
}

/// 名前と別名 → 指しうる用語。長い名前を先に当てる
type Index = Vec<(String, Vec<String>)>;

/// 用語かもしれない所。`refs` はその言葉が指しうる用語
#[derive(Debug, PartialEq)]
struct Candidate {
    start: usize,
    spoken: String,
    refs: Vec<String>,
    /// 名前が似ているだけの言葉（名前・別名・スキルの呼び方ではない）
    guessed: bool,
}

impl Candidate {
    fn end(&self) -> usize {
        self.start + self.spoken.len()
    }
}

impl Decider {
    pub fn new(
        access: Access,
        model: &'static str,
        source_lang: LangCode,
        meter: Arc<Meter>,
    ) -> Self {
        Self {
            access,
            model,
            source_lang,
            meter,
            index: Mutex::new(None),
        }
    }

    /// 文の区切り（用語なら参照）。鍵やギフトコードが断られたら `TranscriptionError`
    pub async fn terms(
        &self,
        http: &reqwest::Client,
        text: &str,
        context: &[String],
        loaded: &Loaded,
    ) -> anyhow::Result<Vec<Span>> {
        let Some(asked) = self.ask(text, context, loaded) else {
            return Ok(vec![Span::Text(text.to_owned())]);
        };
        // OpenAI は Decisions API、ほかの会社は Jev と同じ形の口（OpenRouter も同じ形で中継する）
        let openai = self.access.provider() == Provider::OpenAi;
        let request = if openai {
            http.post(self.access.url("/decisions"))
                .json(&asked.request)
        } else {
            http.post(self.access.url("/systemone"))
                .json(&asked.systemone(self.model))
        };
        let asking = Asking {
            access: &self.access,
            model: self.model,
            meter: &self.meter,
            purpose: Use::Translate,
        };
        let answers = answers(request, &asking).await?;
        Ok(asked.spans(|name| choice(&answers, name)))
    }

    /// 文の候補と、それを確かめる問い合わせ。候補がなければ None（聞かなくてよい）
    pub fn ask(&self, text: &str, context: &[String], loaded: &Loaded) -> Option<Asked> {
        let index = self.index(loaded);
        let candidates = candidates(text, context, &index, &loaded.glossary);
        (!candidates.is_empty()).then(|| Asked {
            request: question(self.model, text, context, &candidates, &loaded.glossary),
            text: text.to_owned(),
            candidates,
        })
    }

    fn index(&self, loaded: &Loaded) -> Arc<Index> {
        let mut cached = self.index.lock().unwrap();
        match &*cached {
            Some((from, index))
                if Arc::ptr_eq(&from.glossary, &loaded.glossary)
                    && Arc::ptr_eq(&from.aliases, &loaded.aliases) =>
            {
                index.clone()
            }
            _ => {
                let index = Arc::new(index(loaded, &self.source_lang));
                *cached = Some((loaded.clone(), index.clone()));
                index
            }
        }
    }
}

/// 判定モデルへの問い合わせの、送り先と数え方
pub(crate) struct Asking<'a> {
    pub access: &'a Access,
    pub model: &'a str,
    pub meter: &'a Meter,
    pub purpose: Use,
}

/// 判定モデルに送って、答え（`answers`）を受け取る。鍵やギフトコードが断られたら `TranscriptionError`
pub(crate) async fn answers(
    request: reqwest::RequestBuilder,
    asking: &Asking<'_>,
) -> anyhow::Result<Value> {
    let res = request
        .bearer_auth(asking.access.token())
        .timeout(TIMEOUT)
        .send()
        .await?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        let reason = share::refusal_reason(&body);
        if let Some(refused) = share::refusal(asking.access, status, reason.as_deref()) {
            return Err(refused.into());
        }
        anyhow::bail!("{status}: {body}");
    }
    let mut res: Value = res.json().await?;
    let tokens = Tokens::from_response(&res["usage"]);
    // OpenRouter は料金を答えに入れる
    match res["usage"]["cost"].as_f64() {
        Some(usd) => asking.meter.billed(asking.purpose, tokens, usd),
        None => asking.meter.tokens(asking.purpose, asking.model, tokens),
    }
    Ok(res["answers"].take())
}

/// `name` の問いで選ばれた値。答えは、問いの配列（OpenAI）か、名前ごとの表（Jev の形）
pub(crate) fn choice(answers: &Value, name: &str) -> Option<String> {
    let answer = match answers.as_array() {
        Some(answers) => answers.iter().find(|a| a["name"] == name)?,
        None => &answers[name],
    };
    answer["choice"].as_str().map(str::to_owned)
}

/// 1 つの文の候補と、Decisions API への問い合わせ
pub struct Asked {
    /// `POST /v1/decisions` の本文。問いは候補ごとに 1 つ
    pub request: Value,
    text: String,
    candidates: Vec<Candidate>,
}

impl Asked {
    /// Jev の形の問い合わせ（`state` と、名前ごとの `questions`、選択肢は `criteria`）
    pub fn systemone(&self, model: &str) -> Value {
        let questions = self.request["questions"].as_array().into_iter().flatten();
        let questions: serde_json::Map<String, Value> = questions
            .filter_map(|q| {
                let choices = q["choices"].as_array().into_iter().flatten();
                let criteria: serde_json::Map<String, Value> = choices
                    .filter_map(|c| Some((c["value"].as_str()?.to_owned(), c["description"].clone())))
                    .collect();
                let question = json!({ "type": "choice", "instructions": q["instructions"], "criteria": criteria });
                Some((q["name"].as_str()?.to_owned(), question))
            })
            .collect();
        json!({ "model": model, "state": self.request["input"], "questions": questions })
    }

    /// 答えを当てはめた、文の区切り。`choice` は問いの名前から、選ばれた値を返す
    pub fn spans(self, choice: impl Fn(&str) -> Option<String>) -> Vec<Span> {
        let chosen = self
            .candidates
            .into_iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let choice = choice(&name(i))?;
                let reference = c.refs.iter().find(|r| **r == choice)?.clone();
                Some((c, reference))
            });
        spans(&self.text, without_echoes(chosen.collect()).into_iter())
    }
}

fn name(i: usize) -> String {
    format!("c{i}")
}

/// 話す言語の名前と別名（2 字以上）を、長い順に並べる
fn index(loaded: &Loaded, source_lang: &str) -> Index {
    let mut by_surface: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
    for (reference, e) in &loaded.glossary.entities {
        let aliases = loaded.aliases.get(reference).into_iter().flatten();
        let names = e.names.get(source_lang).into_iter().chain(aliases);
        for name in names.filter(|n| n.chars().count() >= 2) {
            let refs = by_surface.entry(name).or_default();
            if !refs.contains(reference) {
                refs.push(reference.clone());
            }
        }
    }
    let mut index: Index = by_surface
        .into_iter()
        .map(|(surface, refs)| (surface.to_owned(), refs))
        .collect();
    index.sort_by_key(|(surface, _)| std::cmp::Reverse(surface.len()));
    index
}

/// 文の中の候補（出てくる順）
fn candidates(
    text: &str,
    context: &[String],
    index: &Index,
    glossary: &Glossary,
) -> Vec<Candidate> {
    let mut found = named(text, index);
    // 文と前の発話に出たヒーロー（その言葉が指しうるものを全部）。「ウルト」と番号は、そのスキルから選ばせる
    let before: Vec<Candidate> = context.iter().flat_map(|c| named(c, index)).collect();
    let heroes: Vec<&String> = before
        .iter()
        .chain(&found)
        .flat_map(|c| &c.refs)
        .filter_map(|r| glossary.entities.get_key_value(r))
        .filter(|(_, e)| e.kind == EntityKind::Hero)
        .map(|(r, _)| r)
        .collect();
    let mut at = 0;
    while at < text.len() {
        if let Some(c) = found.iter().find(|c| c.start <= at && at < c.end()) {
            at = c.end();
            continue;
        }
        let rest = &text[at..];
        let Some((slot, len)) = ability_word(rest) else {
            at += rest.chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let refs: Vec<String> = glossary
            .entities
            .iter()
            .filter(|(_, e)| {
                e.ability_slot == Some(slot) && e.hero.as_ref().is_some_and(|h| heroes.contains(&h))
            })
            .map(|(r, _)| r.clone())
            .collect();
        if !refs.is_empty() {
            found.push(Candidate {
                start: at,
                spoken: rest[..len].to_owned(),
                refs,
                guessed: false,
            });
        }
        at += len;
    }
    found.sort_by_key(|c| c.start);
    found
}

/// 名前か別名と同じ所と、名前の似たカタカナの言葉
fn named(text: &str, index: &Index) -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        match index.iter().find(|(s, _)| rest.starts_with(s.as_str())) {
            Some((surface, refs)) => {
                let len = surface.len() + tail(&rest[surface.len()..], surface);
                found.push(Candidate {
                    start: at,
                    spoken: rest[..len].to_owned(),
                    refs: refs.clone(),
                    guessed: false,
                });
                at += len;
            }
            None => at += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    for (start, word) in katakana_words(text, &found) {
        // 「ウルト」は `candidates` が、ヒーローのスキルから選ばせる
        let refs = match ability_word(word) {
            Some(_) => Vec::new(),
            None => similar(word, index),
        };
        if !refs.is_empty() {
            found.push(Candidate {
                start,
                spoken: word.to_owned(),
                refs,
                guessed: true,
            });
        }
    }
    found.sort_by_key(|c| c.start);
    found
}

/// 名前か別名のすぐ後ろに続く、カタカナの言葉の残り（2 字まで）の長さ。別名「アンスト」は「アンストッパ」の
/// 途中までしか当たらないので、言葉の終わりまでを用語にする。3 字以上続くなら別の言葉（「ダイナモウルト」）
fn tail(after: &str, surface: &str) -> usize {
    let is_katakana = |c: char| matches!(c, '\u{30a1}'..='\u{30fc}');
    if !surface.chars().next_back().is_some_and(is_katakana) {
        return 0;
    }
    let run: Vec<char> = after.chars().take_while(|c| is_katakana(*c)).collect();
    match run.len() {
        1..=2 => run.iter().map(|c| c.len_utf8()).sum(),
        _ => 0,
    }
}

/// 候補に入っていないカタカナの言葉（2 字以上）
fn katakana_words<'a>(text: &'a str, found: &[Candidate]) -> Vec<(usize, &'a str)> {
    let mut words = Vec::new();
    let mut at = 0;
    while at < text.len() {
        if let Some(c) = found.iter().find(|c| c.start <= at && at < c.end()) {
            at = c.end();
            continue;
        }
        let len: usize = text[at..]
            .char_indices()
            .take_while(|(i, ch)| {
                matches!(ch, '\u{30a1}'..='\u{30fc}') && !found.iter().any(|c| c.start == at + i)
            })
            .map(|(_, ch)| ch.len_utf8())
            .sum();
        if len == 0 {
            at += text[at..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        let word = &text[at..at + len];
        if word.chars().count() >= 2 {
            words.push((at, word));
        }
        at += len;
    }
    words
}

/// 表記のゆれを寄せる（ヴァ → バ など）
fn kana(text: &str) -> Vec<char> {
    text.replace("ヴァ", "バ")
        .replace("ヴィ", "ビ")
        .replace("ヴェ", "ベ")
        .replace("ヴォ", "ボ")
        .replace('ヴ', "ブ")
        .chars()
        .collect()
}

/// 最長共通部分列の長さ
fn lcs(a: &[char], b: &[char]) -> usize {
    let mut row = vec![0; b.len() + 1];
    for x in a {
        let mut diagonal = 0;
        for (j, y) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = if x == y { diagonal + 1 } else { up.max(row[j]) };
            diagonal = up;
        }
    }
    row[b.len()]
}

/// 言葉の字の 3 分の 2 以上（2 字以上）が順に名前に入っている用語を、似ている順に `MAX_SIMILAR` まで
fn similar(word: &str, index: &Index) -> Vec<String> {
    let word = kana(word);
    let mut scored: Vec<(usize, usize, &String)> = Vec::new();
    for (surface, refs) in index {
        let name = kana(surface);
        let common = lcs(&word, &name);
        if common >= 2 && common * 3 >= word.len() * 2 {
            // 言葉をよく説明し、名前の長さが近いものを先に
            scored.extend(
                refs.iter()
                    .map(|r| (word.len() - common, name.len() - common, r)),
            );
        }
    }
    scored.sort();
    let mut refs: Vec<String> = Vec::new();
    for (_, _, r) in scored {
        if refs.len() < MAX_SIMILAR && !refs.contains(r) {
            refs.push(r.clone());
        }
    }
    refs
}

/// スキルの呼び方（「ウルト」は 4 番、「2」「３番」はその番号）。何番かと、話した言葉の長さ
fn ability_word(rest: &str) -> Option<(u8, usize)> {
    for word in ["ウルト", "アルティメット", "ult"] {
        if rest.starts_with(word) {
            return Some((4, word.len()));
        }
    }
    let first = rest.chars().next()?;
    let slot = match first {
        '1'..='4' => first as u8 - b'0',
        '１'..='４' => (first as u32 - '１' as u32) as u8 + 1,
        _ => return None,
    };
    let after = &rest[first.len_utf8()..];
    let counter = if after.starts_with('番') {
        '番'.len_utf8()
    } else {
        0
    };
    Some((slot, first.len_utf8() + counter))
}

/// 候補ごとに「どの用語か、どれでもないか」を選ばせる問い合わせ
fn question(
    model: &str,
    text: &str,
    context: &[String],
    candidates: &[Candidate],
    glossary: &Glossary,
) -> Value {
    let describe = |reference: &String| {
        let e = &glossary.entities[reference];
        let name = |lang: &str| e.names.get(lang).map_or("", String::as_str);
        let kind = match e.kind {
            EntityKind::Hero => "hero",
            EntityKind::Item => "item",
            EntityKind::Ability => "ability",
        };
        let owner = e
            .hero
            .as_ref()
            .and_then(|h| glossary.entities.get(h)?.names.get("en"))
            .zip(e.ability_slot)
            .map_or(String::new(), |(h, slot)| {
                format!(", ability {slot} of {h}")
            });
        format!("{} ({}): Deadlock {kind}{owner}", name("en"), name("ja"))
    };
    let questions: Vec<Value> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut choices: Vec<Value> = c
                .refs
                .iter()
                .map(|r| json!({ "value": r, "description": describe(r) }))
                .collect();
            choices.push(json!({
                "value": "none",
                "description": "None of these: an ordinary word, part of another word, or it cannot be told which one is meant",
            }));
            json!({
                "type": "choice",
                "name": name(i),
                "instructions": format!(
                    "In the transcript, right after \"{}\", the streamer says \"{}\". What do they mean by it? \
                     Speech recognition may have misheard it, and streamers use nicknames and abbreviations. \
                     An ability is called by its number or \"ult\" (ability 4) after its hero; use the context to tell whose it is.",
                    &text[..c.start], c.spoken
                ),
                "choices": choices,
            })
        })
        .collect();
    json!({
        "model": model,
        "input": format!(
            "Live commentary from a Japanese streamer playing the video game Deadlock, written by speech recognition.\n\
             Previous utterances (context only): {}\nTranscript: {text}",
            json!(context)
        ),
        "questions": questions,
    })
}

/// 名前が似ているだけの言葉が、同じ文でもう呼ばれている用語を指したら、用語にしない
/// （「インファーナスのウルトでスタン入った」の「スタン」は、ウルトの名前「スタンバースト」の一部だが、効果の話）
fn without_echoes(chosen: Vec<(Candidate, String)>) -> Vec<(Candidate, String)> {
    let named: Vec<String> = chosen
        .iter()
        .filter(|(c, _)| !c.guessed)
        .map(|(_, reference)| reference.clone())
        .collect();
    chosen
        .into_iter()
        .filter(|(c, reference)| !c.guessed || !named.contains(reference))
        .collect()
}

/// 選ばれた候補を用語にした区切り。隣り合う同じ用語は 1 つにする
fn spans(text: &str, chosen: impl Iterator<Item = (Candidate, String)>) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut at = 0;
    for (c, reference) in chosen {
        match spans.last_mut() {
            Some(Span::Entity {
                reference: last,
                spoken,
            }) if *last == reference && at == c.start => spoken.push_str(&c.spoken),
            _ => {
                if c.start > at {
                    spans.push(Span::Text(text[at..c.start].to_owned()));
                }
                spans.push(Span::Entity {
                    reference,
                    spoken: c.spoken.clone(),
                });
            }
        }
        at = c.end();
    }
    if at < text.len() {
        spans.push(Span::Text(text[at..].to_owned()));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::Aliases;

    fn loaded() -> Loaded {
        let glossary: Glossary = serde_json::from_value(json!({
            "clientVersion": 1, "languages": ["ja", "en"],
            "entities": {
                "hero:13": { "kind": "hero", "className": "h", "names": { "ja": "ヘイズ", "en": "Haze" } },
                "hero:20": { "kind": "hero", "className": "p", "names": { "ja": "ポケット", "en": "Pocket" } },
                "hero:67": { "kind": "hero", "className": "g", "names": { "ja": "ペイジ", "en": "Paige" } },
                "hero:86": { "kind": "hero", "className": "v", "names": { "ja": "ヴァイオレット", "en": "Violet" } },
                "ability:2": { "kind": "ability", "className": "a", "names": { "ja": "スモークボム", "en": "Smoke Bomb" }, "hero": "hero:13", "abilitySlot": 2 },
                "ability:4": { "kind": "ability", "className": "b", "names": { "ja": "バレットダンス", "en": "Bullet Dance" }, "hero": "hero:13", "abilitySlot": 4 },
                "ability:24": { "kind": "ability", "className": "c", "names": { "ja": "アフリクション", "en": "Affliction" }, "hero": "hero:20", "abilitySlot": 4 },
                "item:1": { "kind": "item", "className": "i", "names": { "ja": "基礎拡張マガジン", "en": "Extended Magazine" } },
                "item:2": { "kind": "item", "className": "j", "names": { "ja": "エコーシャード", "en": "Echo Shard" } },
                "item:3": { "kind": "item", "className": "k", "names": { "ja": "呪いの遺物", "en": "Cursed Relic" } }
            }
        }))
        .unwrap();
        Loaded {
            glossary: Arc::new(glossary),
            requested: Vec::new(),
            root: Default::default(),
            aliases: Arc::new(Aliases::from([
                ("item:1".to_owned(), vec!["拡マガ".to_owned()]),
                ("item:3".to_owned(), vec!["カースドレリック".to_owned()]),
            ])),
        }
    }

    fn found(text: &str, context: &[&str]) -> Vec<(String, Vec<String>)> {
        let loaded = loaded();
        let context: Vec<String> = context.iter().map(|c| (*c).to_owned()).collect();
        candidates(text, &context, &index(&loaded, "ja"), &loaded.glossary)
            .into_iter()
            .map(|c| (c.spoken, c.refs))
            .collect()
    }

    fn refs(list: &[&str]) -> Vec<String> {
        list.iter().map(|r| (*r).to_owned()).collect()
    }

    #[test]
    fn names_and_aliases_are_candidates() {
        assert_eq!(
            found("拡マガと基礎拡張マガジンは同じ。", &[]),
            [
                ("拡マガ".to_owned(), refs(&["item:1"])),
                ("基礎拡張マガジン".to_owned(), refs(&["item:1"])),
            ]
        );
    }

    #[test]
    fn a_name_takes_the_rest_of_its_katakana_word() {
        // 「エコーシャー」まで話した（名前は「エコーシャード」ではなく、別名が途中まで当たる形）
        let mut loaded = loaded();
        loaded.aliases = Arc::new(Aliases::from([(
            "item:2".to_owned(),
            vec!["エコシャ".to_owned()],
        )]));
        let index = index(&loaded, "ja");
        let spoken = |text: &str| -> Vec<String> {
            named(text, &index).into_iter().map(|c| c.spoken).collect()
        };
        assert_eq!(spoken("エコシャー買う。"), ["エコシャー"]);
        // 3 字以上続くなら別の言葉。かなでない字の前では伸ばさない
        assert_eq!(spoken("ヘイズウルト。"), ["ヘイズ"]);
        assert_eq!(spoken("エコシャ買う。"), ["エコシャ"]);
    }

    #[test]
    fn similar_katakana_words_are_candidates() {
        // 略称、名前の一部、聞き違い、表記のゆれ
        assert_eq!(
            found("エコシャ積みたい。", &[]),
            [("エコシャ".to_owned(), refs(&["item:2"]))]
        );
        assert_eq!(
            found("カースは買っとけ。", &[]),
            [("カース".to_owned(), refs(&["item:3"]))]
        );
        assert_eq!(
            found("次のページ。", &[]),
            [("ページ".to_owned(), refs(&["hero:67"]))]
        );
        // 似た名前が 2 つあれば、どちらも選べるようにする（バレットダンスも字が重なる）
        assert_eq!(
            found("バイオレットが気になる。", &[]),
            [("バイオレット".to_owned(), refs(&["hero:86", "ability:4"]))]
        );
        // 似た名前のない言葉は候補にしない
        assert_eq!(found("コメントありがとう。", &[]), []);
    }

    #[test]
    fn ult_and_numbers_are_abilities_of_the_heroes_mentioned() {
        assert_eq!(
            found("ヘイズの2で寝かせてからウルト。", &[]),
            [
                ("ヘイズ".to_owned(), refs(&["hero:13"])),
                ("2".to_owned(), refs(&["ability:2"])),
                ("ウルト".to_owned(), refs(&["ability:4"])),
            ]
        );
        // 前の発話のヒーローも選べる
        assert_eq!(
            found("ウルト避けて。", &["ポケットが来た。", "ヘイズもいる。"]),
            [("ウルト".to_owned(), refs(&["ability:24", "ability:4"]))]
        );
        // ヒーローが分からなければ聞かない
        assert_eq!(found("ウルト溜まった。3番も。", &[]), []);
    }

    #[test]
    fn chosen_candidates_become_spans() {
        let text = "ヘイズの2で寝かせてからウルト。";
        let loaded = loaded();
        let mut all = candidates(text, &[], &index(&loaded, "ja"), &loaded.glossary).into_iter();
        let (hero, two) = (all.next().unwrap(), all.next().unwrap());
        let chosen = [(hero, "hero:13".to_owned()), (two, "ability:2".to_owned())];
        let entity = |reference: &str, spoken: &str| Span::Entity {
            reference: reference.into(),
            spoken: spoken.into(),
        };
        assert_eq!(
            spans(text, chosen.into_iter()),
            [
                entity("hero:13", "ヘイズ"),
                Span::Text("の".into()),
                entity("ability:2", "2"),
                Span::Text("で寝かせてからウルト。".into()),
            ]
        );
    }

    #[test]
    fn a_similar_word_does_not_repeat_a_term_already_named() {
        let loaded = loaded();
        // 「バレット」はバレットダンスに似ている。ウルトがバレットダンスなら、同じ用語を 2 度付けない
        let text = "ヘイズのウルトのバレットが痛い。";
        let found = candidates(text, &[], &index(&loaded, "ja"), &loaded.glossary);
        let spoken: Vec<&str> = found.iter().map(|c| c.spoken.as_str()).collect();
        assert_eq!(spoken, ["ヘイズ", "ウルト", "バレット"]);
        let chosen: Vec<(Candidate, String)> = found
            .into_iter()
            .zip(["hero:13", "ability:4", "ability:4"].map(String::from))
            .collect();
        let kept: Vec<String> = without_echoes(chosen)
            .into_iter()
            .map(|(c, _)| c.spoken)
            .collect();
        assert_eq!(kept, ["ヘイズ", "ウルト"]);
    }

    #[test]
    fn the_question_offers_each_candidate_and_none() {
        let loaded = loaded();
        let text = "ポケットのウルト避けて。";
        let candidates = candidates(text, &[], &index(&loaded, "ja"), &loaded.glossary);
        let body = question(
            "gpt-6-luna",
            text,
            &["前の発話".to_owned()],
            &candidates,
            &loaded.glossary,
        );
        assert_eq!(body["model"], "gpt-6-luna");
        let questions = body["questions"].as_array().unwrap();
        assert_eq!(questions.len(), 2);
        let values: Vec<&str> = questions[1]["choices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["value"].as_str().unwrap())
            .collect();
        assert_eq!(values, ["ability:24", "none"]);
        assert!(
            questions[1]["choices"][0]["description"]
                .as_str()
                .unwrap()
                .contains("ability 4 of Pocket")
        );
    }
}
