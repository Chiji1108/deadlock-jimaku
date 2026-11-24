//! 用語の判定を、PC の中の候補出しと、判定モデルの確かめに分ける（DESIGN.md 9.7）。
//!
//! 判定モデル（OpenAI の Decisions API、TypeSafe AI の Jev、OpenRouter の判定モデル）は文を書かずに選択肢から選ぶだけなので、LLM に辞書の全部を渡して文を区切らせるより
//! ずっと速く安い。ただ文の中から用語を探すことはできないので、候補は PC の中で出す:
//! 辞書の名前と別名の一致、名前の似たカタカナの言葉（略称・名前の一部・聞き違い）、「ウルト」と番号。
//! 候補は多めに出し、本当にその用語か（ふつうの言葉ではないか、どのスキルか）を文ごとに選ばせる。
//! PC の中の LLM（Gemma 4 E4B）にも、同じ問いを選択肢の記号 1 文字で答えさせられる

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use futures_util::future::try_join_all;
use serde_json::{Value, json};

use crate::access::{Access, Server};
use crate::assets::Loaded;
use crate::enrich::Span;
use crate::llama::{self, Llama, Prompt};
use crate::relay;
use crate::secrets::Provider;
use crate::types::{ChromeChoice, ChromeQuestion, ChromeQuestions, EntityKind, Glossary, LangCode};
use crate::usage::{Meter, Tokens, Use};

/// 応答は 0.3 秒ほど。待たせすぎずに、辞書の一致に切り替える
const TIMEOUT: Duration = Duration::from_secs(3);

/// 1 つの言葉に当てる、似た名前の用語の数
const MAX_SIMILAR: usize = 5;

/// PC の中の LLM は、GPU の遅い PC でも待てるようにする（M5 Pro で 0.06 秒ほど）
const LOCAL_TIMEOUT: Duration = Duration::from_secs(5);

/// 選択肢に振る記号。これより多い選択肢は聞かない（候補は `MAX_SIMILAR` までなので、足りる）
const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// PC の中の LLM への指示。問いは 1 つずつ聞く
const LOCAL_SYSTEM: &str = "You answer one multiple-choice question about a Japanese subtitle from a live stream. \
     Read the text and the options carefully, then reply with the letter of the single best option and nothing else.";

/// 候補を確かめさせる相手
enum Judge {
    /// 会社の判定モデル。OpenAI の Decisions API が使えるのは `gpt-6-luna` だけ
    Api {
        access: Access,
        model: &'static str,
    },
    /// 開発者のサーバーの仕事の口（DESIGN.md 17.4）。どのモデルで確かめるかはサーバーが決める
    Server(Server),
    Local(Arc<Llama>),
}

pub struct Decider {
    judge: Judge,
    meter: Arc<Meter>,
    asker: Asker,
}

/// 文から用語の候補を出して、確かめる問いを作る。確かめるのは、会社の判定モデル（`Decider`）か、
/// Chrome の中の Gemini Nano（`chrome::decide`）
pub struct Asker {
    source_lang: LangCode,
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
    /// 名前が似ているだけの言葉で終わる名前の用語（「ビーム」と「アークティックビーム」「レスキュービーム」）。
    /// 複合の名前は終わりが主になるので、それだけで呼ぶことがある。辞書の全部から数える（`refs` は
    /// `MAX_SIMILAR` まで）
    ends: Vec<String>,
    /// その言葉が入っている名前の用語が 1 つだけで、言葉がその名前の終わりにあるとき、その用語（「タレット」と
    /// 「ミニタレット」）。途中にたまたま入っているだけの言葉（「タワー」と「ネコマタワード」）は違う
    sole: Option<String>,
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
            judge: Judge::Api { access, model },
            meter,
            asker: Asker::new(source_lang),
        }
    }

    /// 開発者のサーバーに確かめさせる
    pub fn server(server: Server, source_lang: LangCode, meter: Arc<Meter>) -> Self {
        Self {
            judge: Judge::Server(server),
            meter,
            asker: Asker::new(source_lang),
        }
    }

    /// PC の中の LLM に確かめさせる
    pub fn local(llama: Arc<Llama>, source_lang: LangCode, meter: Arc<Meter>) -> Self {
        Self {
            judge: Judge::Local(llama),
            meter,
            asker: Asker::new(source_lang),
        }
    }

    /// 文の区切り（用語なら参照）。鍵やコードが断られたら `TranscriptionError`
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
        let (access, model) = match &self.judge {
            Judge::Api { access, model } => (access, *model),
            Judge::Server(server) => {
                let (state, questions) = (&asked.request["input"], &asked.request["questions"]);
                let chosen = ask_server(http, server, "/decide", state, questions).await?;
                return Ok(asked.spans(|name| chosen.get(name).cloned()));
            }
            Judge::Local(llama) => {
                let chosen = asked.ask_local(llama, &self.meter).await?;
                return Ok(asked.spans(|name| chosen.get(name).cloned()));
            }
        };
        // OpenAI は Decisions API、ほかの会社は Jev と同じ形の口（OpenRouter も同じ形で中継する）
        let openai = access.provider() == Provider::OpenAi;
        let request = if openai {
            http.post(access.url("/decisions")).json(&asked.request)
        } else {
            http.post(access.url("/systemone"))
                .json(&asked.systemone(model))
        };
        let asking = Asking {
            access,
            model,
            meter: &self.meter,
            purpose: Use::Translate,
        };
        let answers = answers(request, &asking).await?;
        Ok(asked.spans(|name| choice(&answers, name)))
    }

    /// 文の候補と、それを確かめる問い合わせ。候補がなければ None（聞かなくてよい）
    pub fn ask(&self, text: &str, context: &[String], loaded: &Loaded) -> Option<Asked> {
        let model = match &self.judge {
            Judge::Api { model, .. } => model,
            // モデルはサーバーが決める
            Judge::Server(_) => "",
            Judge::Local(_) => llama::GEMMA,
        };
        self.asker.ask(model, text, context, loaded)
    }
}

impl Asker {
    pub fn new(source_lang: LangCode) -> Self {
        Self {
            source_lang,
            index: Mutex::new(None),
        }
    }

    /// 文の候補と、それを確かめる問い合わせ。候補がなければ None（聞かなくてよい）。`model` は会社に送るモデルの名前
    pub fn ask(
        &self,
        model: &str,
        text: &str,
        context: &[String],
        loaded: &Loaded,
    ) -> Option<Asked> {
        let index = self.index(loaded);
        let candidates = candidates(text, context, &index, &loaded.glossary);
        (!candidates.is_empty()).then(|| Asked {
            request: question(model, text, context, &candidates, &loaded.glossary),
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

/// 判定モデルに送って、答え（`answers`）を受け取る。鍵やコードが断られたら `TranscriptionError`
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
        if let Some(refused) = relay::refusal(asking.access, status) {
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

/// Jev と同じ形の口（OpenRouter も同じ形で中継する）への問い合わせ。`questions` は、問いの配列（OpenAI の形）
pub(crate) fn systemone(model: &str, state: &Value, questions: &Value) -> Value {
    let questions = questions.as_array().into_iter().flatten();
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
    json!({ "model": model, "state": state, "questions": questions })
}

/// 開発者のサーバーに選ばせる。`door` は仕事の口（用語は `/decide`、読みは `/read`。サーバーは、仕事ごとにモデルを決める）。
/// `questions` は、問いの配列（名前、指示、選べる値とその説明。OpenAI の形と同じ）。
/// 問いの名前 → 選ばれた値（答えのない問いは入っていない）。コードが断られたら `TranscriptionError`。
/// かかった額は数えない（払うのは開発者）
pub(crate) async fn ask_server(
    http: &reqwest::Client,
    server: &Server,
    door: &str,
    state: &Value,
    questions: &Value,
) -> anyhow::Result<BTreeMap<String, String>> {
    let res = http
        .post(server.url(door))
        .bearer_auth(server.code())
        .timeout(TIMEOUT)
        .json(&json!({ "state": state, "questions": questions }))
        .send()
        .await?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        let reason = relay::refusal_reason(&body);
        if let Some(refused) = relay::code_refusal(status, reason.as_deref()) {
            return Err(refused.into());
        }
        anyhow::bail!("server: {status}: {body}");
    }
    #[derive(serde::Deserialize)]
    struct Answer {
        choices: BTreeMap<String, String>,
    }
    Ok(res.json::<Answer>().await?.choices)
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
    /// Chrome のページに送る形（`id` は発話）。問いごとに、選択肢とその説明
    pub fn for_chrome(&self, id: String) -> ChromeQuestions {
        let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
        let questions = self.request["questions"].as_array().into_iter().flatten();
        let questions = questions.map(|q| ChromeQuestion {
            name: text(&q["name"]),
            instructions: text(&q["instructions"]),
            choices: (q["choices"].as_array().into_iter().flatten())
                .map(|c| ChromeChoice {
                    value: text(&c["value"]),
                    description: text(&c["description"]),
                })
                .collect(),
        });
        ChromeQuestions {
            id,
            state: text(&self.request["input"]),
            questions: questions.collect(),
        }
    }

    /// PC の中の LLM に、問いを 1 つずつ同時に聞く。選択肢に記号を振り、記号 1 文字だけを答えさせる
    /// （文法で、ほかの字を書けなくする）。問いの名前 → 選ばれた値。1 つでも答えが返らなければ失敗にする
    /// （答えのない候補を「用語ではない」として扱うと、辞書の一致より悪くなる）
    async fn ask_local(
        &self,
        llama: &Llama,
        meter: &Meter,
    ) -> anyhow::Result<BTreeMap<String, String>> {
        let state = self.request["input"].as_str().unwrap_or_default();
        let questions = self.request["questions"].as_array().into_iter().flatten();
        let asked = questions.map(|q| async move {
            let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
            let choices: Vec<&Value> = q["choices"].as_array().into_iter().flatten().collect();
            let letters: Vec<char> = LETTERS.chars().take(choices.len()).collect();
            let listed: Vec<String> = (letters.iter().zip(&choices))
                .map(|(letter, c)| format!("{letter}. {}", text(&c["description"])))
                .collect();
            let either: Vec<String> = letters.iter().map(|l| format!("\"{l}\"")).collect();
            let user = format!(
                "{state}\n\nQuestion: {}\n\nOptions:\n{}",
                text(&q["instructions"]),
                listed.join("\n")
            );
            let prompt = Prompt {
                system: LOCAL_SYSTEM,
                user: &user,
                grammar: &format!("root ::= {}", either.join(" | ")),
                max_tokens: 2,
                timeout: LOCAL_TIMEOUT,
            };
            let (answer, tokens) = llama.complete(&prompt).await?;
            meter.tokens(Use::Translate, llama::GEMMA, tokens);
            let chosen = letters.iter().position(|l| answer.starts_with(*l));
            let chosen = chosen.with_context(|| format!("no choice: {answer:?}"))?;
            anyhow::Ok((text(&q["name"]), text(&choices[chosen]["value"])))
        });
        try_join_all(asked).await.map(BTreeMap::from_iter)
    }

    /// Jev の形の問い合わせ（`state` と、名前ごとの `questions`、選択肢は `criteria`）
    pub fn systemone(&self, model: &str) -> Value {
        systemone(model, &self.request["input"], &self.request["questions"])
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
        // 別名（用語は LLM が作ったもの、人は本人が書いた呼ばれ方）
        let aliases = loaded.aliases.get(reference).into_iter().flatten();
        let names = e.names.get(source_lang).into_iter().chain(aliases);
        let names = names.chain(&e.aliases);
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
    // 文と前の発話に出たヒーロー（その言葉が指しうるものを全部）。「ウルト」は、そのスキルから選ばせる。
    // 誰の名前も出ていなければ、誰のスキルか分からないので候補にしない
    let before: Vec<Candidate> = context.iter().flat_map(|c| named(c, index)).collect();
    let said = before.iter().chain(&found).flat_map(|c| &c.refs);
    let heroes: Vec<&String> = said
        .filter_map(|r| glossary.entities.get_key_value(r))
        .filter(|(_, e)| e.kind == EntityKind::Hero)
        .map(|(r, _)| r)
        .collect();
    // 名前が似ているだけの言葉は、スキルなら、持ち主のヒーローの名前が出ているときだけ候補にする
    // （「ダッシュ」は動きの名前で、インファーナスの話をしているときだけ「フレイムダッシュ」かもしれない）。
    // 「ウルト」や番号と同じ決まり。誰のスキルか分からないのに判定モデルに任せると、スキルを選ぶことがある。
    // 基本用語は、似ているだけでは候補にしない。もともと一般の言葉なので、その一部はさらに一般の言葉になる
    // （「ソウル」は別名「ソウルアーン」の一部で、判定モデルは「魂の壺」を選んだ）
    //
    // 言葉が名前の終わりにそのまま入っているときは、そういう用語がいくつあるかで決める:
    // - その言葉が入る名前が 1 つだけなら（`Candidate::sole`）、スキルでも持ち主を求めない
    //   （「タレット」が入る名前は「ミニタレット」だけ）
    // - その言葉で終わる名前がいくつもあり（`Candidate::ends`）、その中のスキルの持ち主が誰も出ていなければ、
    //   どれのことか決められないので、そのどれも候補にしない（「ビーム」は「アークティックビーム」「ハイパービーム」と
    //   アイテムの「レスキュービーム」。持ち主で外したスキルの代わりに、残ったアイテムを選ばせることになる）。
    //   名前の頭や聞き違いで当たった用語は残す（「ヴィン」は「ケルビン」の終わりでもあるが、「ヴィンディクタ」の頭）
    let ability = |r: &String| glossary.entities[r].kind == EntityKind::Ability;
    let owned = |r: &String| {
        let hero = glossary.entities[r].hero.as_ref();
        hero.is_some_and(|h| heroes.contains(&h))
    };
    for candidate in found.iter_mut().filter(|c| c.guessed) {
        let (ends, sole) = (&candidate.ends, &candidate.sole);
        let abilities = || ends.iter().filter(|r| ability(r));
        let undecided = ends.len() > 1 && abilities().next().is_some() && !abilities().any(owned);
        candidate.refs.retain(|r| match glossary.entities[r].kind {
            _ if undecided && ends.contains(r) => false,
            EntityKind::Ability => owned(r) || sole.as_ref() == Some(r),
            EntityKind::Term => false,
            _ => true,
        });
    }
    found.retain(|c| !c.refs.is_empty());
    let mut at = 0;
    while at < text.len() {
        if let Some(c) = found.iter().find(|c| c.start <= at && at < c.end()) {
            at = c.end();
            continue;
        }
        let rest = &text[at..];
        let Some((slot, len, number)) = ability_word(rest) else {
            at += rest.chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let owners: Vec<&String> = if number {
            // 数字は普通の言葉にも出る（「1人」「1番いい」「10秒」）ので、ヒーローの名前のすぐ後ろ
            // （「ヘイズの2」）だけ、そのヒーローのスキルにする。外すと、翻訳にスキルの名前が入ってしまう
            let after_name = |c: &&Candidate| matches!(text.get(c.end()..at), Some("" | "の"));
            let numeral = |c: char| c.is_numeric() || "〇一二三四五六七八九十百千万".contains(c);
            let more_digits = rest[len..].starts_with(numeral);
            found
                .iter()
                .filter(|c| after_name(c) && !more_digits)
                .flat_map(|c| &c.refs)
                .filter(|r| heroes.contains(r))
                .collect()
        } else {
            heroes.clone()
        };
        let refs: Vec<String> = glossary
            .entities
            .iter()
            .filter(|(_, e)| {
                e.ability_slot == Some(slot) && e.hero.as_ref().is_some_and(|h| owners.contains(&h))
            })
            .map(|(r, _)| r.clone())
            .collect();
        if !refs.is_empty() {
            found.push(Candidate {
                start: at,
                spoken: rest[..len].to_owned(),
                refs,
                guessed: false,
                ends: Vec::new(),
                sole: None,
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
                    ends: Vec::new(),
                    sole: None,
                });
                at += len;
            }
            None => at += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    for (start, word) in katakana_words(text, &found) {
        // 「ウルト」は `candidates` が、ヒーローのスキルから選ばせる
        let (refs, ends, sole) = match ability_word(word) {
            Some(_) => Default::default(),
            None => similar(word, index),
        };
        if !refs.is_empty() {
            found.push(Candidate {
                start,
                spoken: word.to_owned(),
                refs,
                guessed: true,
                ends,
                sole,
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

/// 名前の似た用語を、似ている順に `MAX_SIMILAR` まで。似ているのは、次のどちらか:
/// - 名前の一部（略称や、名前の後ろ半分。「カート」と「キャリーカート」）: 言葉がそのまま名前の中に続けて入っている
/// - 名前全体の聞き違い（「ページ」と「ペイジ」、「エコシャ」と「エコーシャード」）: 言葉の字の 3 分の 2 以上
///   （2 字以上）が順に名前に入っていて、それが名前の半分以上を占める
///
/// 長い名前のあちこちの字にたまたま当たるだけの言葉（「プレイ」と「グレイ・タロン」、「デッドロック」と
/// 「デベロップ・クイーン」）は、どちらでもないので候補にしない
///
/// 2 つ目は `Candidate::ends`、3 つ目は `Candidate::sole`
fn similar(word: &str, index: &Index) -> (Vec<String>, Vec<String>, Option<String>) {
    let word = kana(word);
    let mut scored: Vec<(usize, usize, &String)> = Vec::new();
    let mut parts: Vec<&String> = Vec::new();
    let mut ends: Vec<String> = Vec::new();
    for (surface, refs) in index {
        let name = kana(surface);
        let part = name.windows(word.len()).any(|w| w == word.as_slice());
        let common = if part { word.len() } else { lcs(&word, &name) };
        let whole = word.first() == name.first()
            && common * 3 >= word.len() * 2
            && common * 2 >= name.len();
        if common >= 2 && part {
            let ending = name.ends_with(&word);
            for r in refs {
                if !parts.contains(&r) {
                    parts.push(r);
                }
                if ending && !ends.contains(r) {
                    ends.push(r.clone());
                }
            }
        }
        if common >= 2 && (part || whole) {
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
    let sole = match &parts[..] {
        [only] if ends.contains(only) => Some((*only).clone()),
        _ => None,
    };
    (refs, ends, sole)
}

/// スキルの呼び方（「ウルト」は 4 番、「2」「３番」「二番」はその番号）。何番か、話した言葉の長さ、数字で呼んだか。
/// 漢数字の「二」〜「四」も受ける（数える言葉のない「ヘイズの二」は、`tidy.rs` が算用数字にしない）
fn ability_word(rest: &str) -> Option<(u8, usize, bool)> {
    for word in ["ウルト", "アルティメット", "ult"] {
        if rest.starts_with(word) {
            return Some((4, word.len(), false));
        }
    }
    let first = rest.chars().next()?;
    let slot = match first {
        '1'..='4' => first as u8 - b'0',
        '１'..='４' => (first as u32 - '１' as u32) as u8 + 1,
        // 「一」は受けない（「ヘイズの一番いいところ」の「一番」は、ほぼ「いちばん」）
        '二' => 2,
        '三' => 3,
        '四' => 4,
        _ => return None,
    };
    let after = &rest[first.len_utf8()..];
    let counter = if after.starts_with('番') {
        '番'.len_utf8()
    } else {
        0
    };
    Some((slot, first.len_utf8() + counter, true))
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
            EntityKind::Term => "game term",
            // ゲームの用語ではないので、別の言い方で説明する
            EntityKind::Person => {
                return format!(
                    "{} ({}): a person, the streamer or someone they play with",
                    name("en"),
                    name("ja")
                );
            }
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
                "hero:17": { "kind": "hero", "className": "g", "names": { "ja": "グレイ・タロン", "en": "Grey Talon" } },
                "ability:2": { "kind": "ability", "className": "a", "names": { "ja": "スモークボム", "en": "Smoke Bomb" }, "hero": "hero:13", "abilitySlot": 2 },
                "ability:4": { "kind": "ability", "className": "b", "names": { "ja": "バレットダンス", "en": "Bullet Dance" }, "hero": "hero:13", "abilitySlot": 4 },
                "ability:24": { "kind": "ability", "className": "c", "names": { "ja": "アフリクション", "en": "Affliction" }, "hero": "hero:20", "abilitySlot": 4 },
                "item:1": { "kind": "item", "className": "i", "names": { "ja": "基礎拡張マガジン", "en": "Extended Magazine" } },
                "item:2": { "kind": "item", "className": "j", "names": { "ja": "エコーシャード", "en": "Echo Shard" } },
                "item:3": { "kind": "item", "className": "k", "names": { "ja": "呪いの遺物", "en": "Cursed Relic" } },
                "item:4": { "kind": "item", "className": "l", "names": { "ja": "ベールウォーカー", "en": "Veil Walker" } },
                "hero:12": { "kind": "hero", "className": "k", "names": { "ja": "ケルビン", "en": "Kelvin" } },
                "ability:8": { "kind": "ability", "className": "t", "names": { "ja": "ミニタレット", "en": "Mini Turret" }, "hero": "hero:8", "abilitySlot": 2 },
                "ability:12": { "kind": "ability", "className": "r", "names": { "ja": "アークティックビーム", "en": "Arctic Beam" }, "hero": "hero:12", "abilitySlot": 3 },
                "item:5": { "kind": "item", "className": "m", "names": { "ja": "レスキュービーム", "en": "Rescue Beam" } },
                "term:8": { "kind": "term", "className": "w", "names": { "ja": "ウォーカー", "en": "Walker" } },
                "term:9": { "kind": "term", "className": "u", "names": { "ja": "魂の壺", "en": "Soul Urn" } }
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
                (
                    "term:9".to_owned(),
                    vec![
                        "壺".to_owned(),
                        "ウルン".to_owned(),
                        "ソウルアーン".to_owned(),
                    ],
                ),
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

    /// 開発者のサーバーに同じ問いを送り、選ばれた値で用語が決まる。モデルは送らない（サーバーが決める）
    #[tokio::test]
    async fn the_server_picks_among_the_candidates() {
        use axum::http::StatusCode;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let answer = |headers: axum::http::HeaderMap, axum::Json(asked): axum::Json<Value>| async move {
            let code = headers["authorization"].to_str().unwrap().to_owned();
            if code != "Bearer good" {
                let error = code.trim_start_matches("Bearer ");
                return (StatusCode::FORBIDDEN, axum::Json(json!({ "error": error })));
            }
            assert!(asked["state"].as_str().unwrap().contains("拡マガを取る"));
            assert!(asked.get("model").is_none());
            // どの問いにも、最初の値を答える
            let questions = asked["questions"].as_array().unwrap().iter();
            let choices: serde_json::Map<String, Value> = questions
                .map(|q| {
                    (
                        q["name"].as_str().unwrap().to_owned(),
                        q["choices"][0]["value"].clone(),
                    )
                })
                .collect();
            (StatusCode::OK, axum::Json(json!({ "choices": choices })))
        };
        let router = axum::Router::new().route("/v2/decide", axum::routing::post(answer));
        tokio::spawn(async move { axum::serve(listener, router).await });
        let meter = Arc::new(Meter::new(
            std::env::temp_dir().join("decide_server_test.json"),
        ));
        let decider = |code: &str| {
            Decider::server(Server::new(&api, code.into()), "ja".into(), meter.clone())
        };
        let http = reqwest::Client::new();
        let loaded = loaded();
        let good = decider("good");
        let spans = good.terms(&http, "拡マガを取る", &[], &loaded).await;
        let spans = spans.unwrap();
        assert!(
            matches!(&spans[0], Span::Entity { reference, .. } if reference == "item:1"),
            "{spans:?}"
        );
        // コードが断られたら、字幕を止める理由にする。その仕事だけを止めているときは、その文だけ諦める
        for (code, stops) in [("revoked", true), ("limit", true), ("stopped", false)] {
            let refused = decider(code);
            let failed = refused.terms(&http, "拡マガを取る", &[], &loaded).await;
            let failed = failed.unwrap_err();
            let stopped = failed.is::<crate::types::TranscriptionError>();
            assert_eq!(stopped, stops, "{code}");
        }
    }

    /// Chrome のページ（Gemini Nano）に同じ問いを送り、選ばれた選択肢で用語が決まる
    #[tokio::test]
    async fn the_chrome_page_answers_the_same_questions() {
        use crate::chrome;
        use crate::settings::Settings;
        use crate::store::{ChromeEvent, Store};
        use crate::types::{ChromeCommand, Decided};

        let store = Store::new(Ok(Settings::default()), std::path::PathBuf::new());
        let asked = Asker::new("ja".into())
            .ask("", "拡マガを取る", &[], &loaded())
            .unwrap();
        // ページが開いていなければ聞かない
        assert_eq!(chrome::decide(&store, &asked, "1").await, None);

        let mut page = store.subscribe_chrome_commands();
        let answering = store.clone();
        tokio::spawn(async move {
            let Ok(ChromeCommand::Decide(task)) = page.recv().await else {
                panic!("a question")
            };
            assert!(task.state.contains("拡マガを取る"));
            // どの問いにも、最初の選択肢を答える
            let choices = task.questions.iter();
            let choices = choices.map(|q| (q.name.clone(), q.choices[0].value.clone()));
            answering.send_chrome_event(ChromeEvent::Decided(Decided {
                id: task.id,
                choices: choices.collect(),
            }));
        });
        let choices = chrome::decide(&store, &asked, "1").await.unwrap();
        // 答えのない問いがあれば使わない（辞書の一致まで消さない）
        let both = Asker::new("ja".into())
            .ask("", "拡マガとヘイズ", &[], &loaded())
            .unwrap();
        let mut page = store.subscribe_chrome_commands();
        let answering = store.clone();
        tokio::spawn(async move {
            let Ok(ChromeCommand::Decide(task)) = page.recv().await else {
                panic!("a question")
            };
            assert_eq!(task.questions.len(), 2);
            let first = &task.questions[0];
            answering.send_chrome_event(ChromeEvent::Decided(Decided {
                id: task.id,
                choices: [(first.name.clone(), first.choices[0].value.clone())].into(),
            }));
        });
        assert_eq!(chrome::decide(&store, &both, "2").await, None);
        let spans = asked.spans(|name| choices.get(name).cloned());
        assert_eq!(
            spans[0],
            Span::Entity {
                reference: "item:1".into(),
                spoken: "拡マガ".into()
            }
        );
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
            found("バイオレットが気になる。", &["ヘイズもいる。"]),
            [("バイオレット".to_owned(), refs(&["hero:86", "ability:4"]))]
        );
        // 名前が似ているだけのスキルは、持ち主のヒーローの名前が出ていなければ候補にしない
        assert_eq!(
            found("バイオレットが気になる。", &[]),
            [("バイオレット".to_owned(), refs(&["hero:86"]))]
        );
        assert_eq!(found("スモークで逃げる。", &[]), []);
        assert_eq!(
            found("ヘイズのスモークで逃げる。", &[])[1],
            ("スモーク".to_owned(), refs(&["ability:2"]))
        );
        // 名前そのものは、持ち主が出ていなくても候補にする
        assert_eq!(
            found("スモークボムで逃げる。", &[]),
            [("スモークボム".to_owned(), refs(&["ability:2"]))]
        );
        // 似た名前のない言葉は候補にしない
        assert_eq!(found("コメントありがとう。", &[]), []);
        // 名前の一部は、そのまま続けて入っていれば候補にする
        assert_eq!(
            found("グレイが飛んでる。", &[]),
            [("グレイ".to_owned(), refs(&["hero:17"]))]
        );
        assert_eq!(
            found("タロンに撃たれた。", &[]),
            [("タロン".to_owned(), refs(&["hero:17"]))]
        );
        // 長い名前のあちこちの字に当たるだけの言葉（「プレイ」と「グレイ・タロン」）は候補にしない。
        // 名前全体の聞き違いは、初めの字が同じものだけ（「プレイ」と「ペイジ」は違う）
        assert_eq!(found("どうやってプレイするの。", &[]), []);
        assert_eq!(found("クレイが飛んでる。", &[]), []);
    }

    #[test]
    fn a_word_that_ends_names_is_decided_by_how_many_names_it_ends() {
        // その言葉が入る名前が 1 つだけなら、持ち主のヒーローの名前が出ていなくても、スキルを候補にする
        assert_eq!(
            found("タレット置かれてる。", &[]),
            [("タレット".to_owned(), refs(&["ability:8"]))]
        );
        // 名前の頭にあるだけの言葉は、今までどおり持ち主が要る（「スモーク」と「スモークボム」）
        assert_eq!(found("スモークで逃げる。", &[]), []);
        // その言葉で終わる名前がいくつもあり、スキルの持ち主が出ていなければ、アイテムも候補にしない
        assert_eq!(found("ビーム当たらない。", &[]), []);
        // 持ち主が出ていれば、そのスキルとアイテムから選ばせる
        assert_eq!(
            found("ケルビンのビームで止められた。", &[])[1],
            ("ビーム".to_owned(), refs(&["item:5", "ability:12"]))
        );
    }

    #[test]
    fn a_basic_term_is_not_taken_for_an_item_that_contains_it() {
        // 辞書に「ウォーカー」がなかったころは、名前の一部が重なる「ベールウォーカー」が候補になった
        assert_eq!(
            found("ウォーカー折れた。", &[]),
            [("ウォーカー".to_owned(), refs(&["term:8"]))]
        );
        assert_eq!(
            found("ベールウォーカー買う。", &[]),
            [("ベールウォーカー".to_owned(), refs(&["item:4"]))]
        );
        // 別名も、ほかの用語と同じく候補になる（本当にそのことかは、判定のモデルが決める）
        assert_eq!(
            found("ウルン運んで。", &[]),
            [("ウルン".to_owned(), refs(&["term:9"]))]
        );
        // 1 字の別名は、ほかの言葉の中にも出るので候補にしない（`index`）
        assert_eq!(found("壺運んで。", &[]), []);
        // 名前や別名に似ているだけの言葉は、基本用語の候補にしない（「ソウル」は「ソウルアーン」の一部）
        assert_eq!(found("ソウル差が5000ある。", &[]), []);
        assert_eq!(
            found("ソウルアーン運んで。", &[]),
            [("ソウルアーン".to_owned(), refs(&["term:9"]))]
        );
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
    fn numbers_are_abilities_only_right_after_a_hero() {
        let spoken = |text: &str, context: &[&str]| {
            let loaded = loaded();
            let context: Vec<String> = context.iter().map(|c| (*c).to_owned()).collect();
            candidates(text, &context, &index(&loaded, "ja"), &loaded.glossary)
                .into_iter()
                .map(|c| c.spoken)
                .collect::<Vec<_>>()
        };
        assert_eq!(spoken("ヘイズの2番で寝かせる。", &[]), ["ヘイズ", "2番"]);
        assert_eq!(spoken("ヘイズの二で寝かせる。", &[]), ["ヘイズ", "二"]);
        assert_eq!(spoken("ヘイズの二十発。", &[]), ["ヘイズ"]);
        assert_eq!(spoken("ヘイズの一番いいところ。", &[]), ["ヘイズ"]);
        assert_eq!(spoken("ヘイズ2で寝かせる。", &[]), ["ヘイズ", "2"]);
        // 数字だけでは拾わない（普通の言葉にも出る）
        assert_eq!(spoken("1人で来てる。2番ある。", &[]), [] as [&str; 0]);
        assert_eq!(
            spoken("それが1番いい。", &["ヘイズもいる。"]),
            [] as [&str; 0]
        );
        assert_eq!(spoken("ヘイズが1人で来てる。", &[]), ["ヘイズ"]);
        assert_eq!(spoken("ヘイズの20秒。", &[]), ["ヘイズ"]);
        // 誰の名前も出ていない「ウルト」は、誰のスキルか分からないので拾わない
        assert_eq!(spoken("ウルト使うよ。", &[]), [] as [&str; 0]);
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
