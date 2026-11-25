//! 翻訳専門のモデル（Qwen-MT-Flash、Hy-MT2）か、汎用の LLM（Qwen3.8 Flash、GPT-6 Luna、Gemma 4 26B）か、PC の中の LLM
//! （Gemma 4 E4B）で訳す（DESIGN.md 9.7）。開発者のサーバーに頼むときは、文と翻訳先だけを送り、モデルと頼み方は
//! サーバーが決める（DESIGN.md 17.4。同じモデルの頼み方は、server/src/tasks.ts にもある）。
//!
//! 用語は先に決めてあるので、英語名に置き換えて `⟦Haze⟧` と囲んだ文を渡し、訳文に残った名前を用語に戻す。
//! 翻訳先ごとに 1 回ずつ、同時に聞く（1 回の応答は 0.2〜0.3 秒）。括弧を外したり引用符に替えたりすることが
//! あるので、括弧がなくても英語名があれば用語にする。名前まで訳された用語は、ふつうの文字のまま出る。
//! PC の中の LLM には、括弧の中にその文の用語の名前しか書けないよう、文法で縛る（縛らないと名前を崩す）

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::future::join_all;
use serde_json::{Value, json};

use crate::access::{Access, Server};
use crate::enrich::Span;
use crate::lang;
use crate::llama::{self, Llama, Prompt};
use crate::relay;
use crate::secrets::Provider;
use crate::types::{Glossary, LangCode, Token, TranscriptionError, Translations};
use crate::usage::{Meter, Tokens, Use};

/// 応答は 0.3 秒ほど（OpenRouter の Hy-MT2 は 0.8 秒ほど）。送り直しを入れても、確定の字幕を 10 秒より長くは待たせない
const TIMEOUT: Duration = Duration::from_secs(2);

/// 汎用の LLM は 1〜2 秒かかる（遅いときは 3 秒ほど）
const LLM_TIMEOUT: Duration = Duration::from_secs(4);

/// 翻訳専門のモデルか（そうでなければ、汎用の LLM に指示文を付けて訳させる）
fn dedicated(model: &str) -> bool {
    model.starts_with("qwen-mt-") || model.contains("hy-mt")
}

/// 回数の上限（429）で断られたら、待って送り直す（訳が遅れても、抜けるよりよい）。上限の値は公開されておらず、
/// 続けて送ると 1 分に 100 回ほどで断られた（実測）
const RETRY_AFTER: [Duration; 2] = [Duration::from_millis(700), Duration::from_millis(1500)];

/// 訳し方の指示。これがないと、括弧を外すことが多い（実測）
const DOMAINS: &str = "Casual live commentary by a streamer playing the video game Deadlock, like gamers talk. \
     Words in ⟦ ⟧ are game terms (names of heroes, items and abilities); \
     keep them exactly as they are, with the brackets.";

/// PC の中の LLM は、GPU の遅い PC でも待てるようにする（M5 Pro で 0.3 秒ほど）
const LOCAL_TIMEOUT: Duration = Duration::from_secs(5);

/// サーバーは、翻訳先をまとめて 1 回で受け、回数の上限で断られたら中で待って送り直す（いちばん長くて 8 秒ほど）
const SERVER_TIMEOUT: Duration = Duration::from_secs(9);

/// 訳を頼む相手
enum Backend {
    Api {
        access: Access,
        model: &'static str,
    },
    /// 開発者のサーバーの、翻訳の口
    Server(Server),
    Local(Arc<Llama>),
}

pub struct Translator {
    backend: Backend,
    source_lang: LangCode,
    targets: Vec<LangCode>,
    meter: Arc<Meter>,
}

impl Translator {
    pub fn new(
        access: Access,
        model: &'static str,
        source_lang: LangCode,
        targets: Vec<LangCode>,
        meter: Arc<Meter>,
    ) -> Self {
        Self {
            backend: Backend::Api { access, model },
            source_lang,
            targets,
            meter,
        }
    }

    /// 開発者のサーバーに訳させる（モデルはサーバーが決める）
    pub fn server(
        server: Server,
        source_lang: LangCode,
        targets: Vec<LangCode>,
        meter: Arc<Meter>,
    ) -> Self {
        Self {
            backend: Backend::Server(server),
            source_lang,
            targets,
            meter,
        }
    }

    /// PC の中の LLM に訳させる
    pub fn local(
        llama: Arc<Llama>,
        source_lang: LangCode,
        targets: Vec<LangCode>,
        meter: Arc<Meter>,
    ) -> Self {
        Self {
            backend: Backend::Local(llama),
            source_lang,
            targets,
            meter,
        }
    }

    /// 用語を決めた文を、翻訳先ごとに訳す。訳せなかった言語は含めない。鍵が断られたら `TranscriptionError`
    pub async fn translate(
        &self,
        http: &reqwest::Client,
        spans: &[Span],
        glossary: Option<&Glossary>,
    ) -> anyhow::Result<Translations> {
        let (source, terms) = tagged(spans, glossary);
        let (source, terms) = (&source, &terms);
        let named = |target: &LangCode, text: anyhow::Result<String>| Ok((target.clone(), text?));
        // 会社と PC の中の LLM には、翻訳先ごとに 1 回ずつ、同時に聞く。サーバーには、まとめて 1 回
        let translated: Vec<anyhow::Result<(LangCode, String)>> = match &self.backend {
            Backend::Api { access, model } => {
                let asked = self.targets.iter().map(|target| async move {
                    named(target, self.ask(http, access, model, source, target).await)
                });
                join_all(asked).await
            }
            Backend::Local(llama) => {
                let asked = self.targets.iter().map(|target| async move {
                    named(target, self.ask_local(llama, source, target, terms).await)
                });
                join_all(asked).await
            }
            Backend::Server(server) => match self.ask_server(http, server, source).await {
                Ok(translated) => translated.into_iter().map(Ok).collect(),
                Err(e) => vec![Err(e)],
            },
        };
        let mut translations = Translations::new();
        for translated in translated {
            match translated {
                Ok((target, text)) => {
                    let tokens = tokens(&text, terms);
                    if !tokens.is_empty() {
                        translations.insert(target, tokens);
                    }
                }
                // 断られた鍵は、どの言語でも送り直しても断られる
                Err(e) if e.is::<TranscriptionError>() => return Err(e),
                Err(e) => tracing::warn!("translation: {e:#}"),
            }
        }
        Ok(translations)
    }

    /// サーバーに、翻訳先をまとめて頼む。訳せなかった翻訳先は、答えに入っていない。その仕事だけを止めているときは、
    /// 訳なしで続ける（途中の字幕が Chrome なら、Chrome の途中の訳が残る。`subtitles::settled`）。
    /// かかった額は数えない（払うのは開発者）
    async fn ask_server(
        &self,
        http: &reqwest::Client,
        server: &Server,
        source: &str,
    ) -> anyhow::Result<Vec<(LangCode, String)>> {
        let body = json!({ "text": source, "source": self.source_lang, "targets": self.targets });
        let res = http
            .post(server.url("/translate"))
            .bearer_auth(server.code())
            .timeout(SERVER_TIMEOUT)
            .json(&body)
            .send()
            .await?;
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            let reason = relay::refusal_reason(&body);
            if let Some(refused) = relay::code_refusal(status, reason.as_deref()) {
                return Err(refused.into());
            }
            if reason.as_deref() == Some(relay::STOPPED) {
                tracing::warn!("translation: the server has stopped translating");
                return Ok(Vec::new());
            }
            anyhow::bail!("server: {status}: {body}");
        }
        #[derive(serde::Deserialize)]
        struct Answer {
            translations: std::collections::BTreeMap<LangCode, String>,
        }
        let answer: Answer = res.json().await?;
        Ok(answer.translations.into_iter().collect())
    }

    async fn ask_local(
        &self,
        llama: &Llama,
        source: &str,
        target: &str,
        terms: &[(String, String)],
    ) -> anyhow::Result<String> {
        let system = format!(
            "Translate the {} text into {}. {DOMAINS} Reply with the translation only.",
            lang::english_name(&self.source_lang),
            lang::english_name(target),
        );
        let prompt = Prompt {
            system: &system,
            user: source,
            grammar: &grammar(terms.iter().map(|(name, _)| name.as_str())),
            max_tokens: 200,
            timeout: LOCAL_TIMEOUT,
        };
        let (text, tokens) = llama.complete(&prompt).await?;
        self.meter.tokens(Use::Translate, llama::GEMMA, tokens);
        Ok(text)
    }

    async fn ask(
        &self,
        http: &reqwest::Client,
        access: &Access,
        model: &str,
        source: &str,
        target: &str,
    ) -> anyhow::Result<String> {
        let body = match access.provider() {
            // 汎用の LLM。PC の中の LLM と同じ指示で、推論はさせない（させると 2 秒を超える）
            provider if !dedicated(model) => {
                let system = format!(
                    "Translate the {} text into {}. {DOMAINS} Reply with the translation only.",
                    lang::english_name(&self.source_lang),
                    lang::english_name(target),
                );
                let mut body = json!({
                    "model": model,
                    "messages": [
                        { "role": "system", "content": system },
                        { "role": "user", "content": source },
                    ],
                });
                match provider {
                    Provider::Qwen => body["enable_thinking"] = json!(false),
                    // OpenRouter は、同じモデルをいくつかの会社に回す。送った文を残さない会社だけに回させる
                    Provider::OpenRouter => {
                        body["reasoning"] = json!({ "enabled": false });
                        body["provider"] = json!({ "data_collection": "deny" });
                        body["max_tokens"] = json!(512);
                    }
                    _ => body["reasoning_effort"] = json!("none"),
                }
                body
            }
            // Hy-MT2。system はなく、モデルの説明にある「背景を添えて訳す」指示文で送る。設定は説明の勧める値
            Provider::OpenRouter => json!({
                "model": model,
                "messages": [{ "role": "user", "content": format!(
                    "[Background Information]\n{DOMAINS}\n\nPlease translate the following text into {}, \
                     taking the provided background information into consideration.\n\n[Source Text]\n{source}",
                    lang::english_name(target),
                ) }],
                "temperature": 0.7,
                "top_p": 1.0,
                "max_tokens": 512,
            }),
            _ => json!({
                "model": model,
                "messages": [{ "role": "user", "content": source }],
                "translation_options": {
                    "source_lang": code(&self.source_lang),
                    "target_lang": code(target),
                    "domains": DOMAINS,
                },
            }),
        };
        let send = || {
            http.post(access.url("/chat/completions"))
                .bearer_auth(access.token())
                .timeout(match dedicated(model) {
                    true => TIMEOUT,
                    false => LLM_TIMEOUT,
                })
                .json(&body)
                .send()
        };
        let mut res = send().await?;
        for wait in RETRY_AFTER {
            if res.status() != reqwest::StatusCode::TOO_MANY_REQUESTS {
                break;
            }
            tokio::time::sleep(wait).await;
            res = send().await?;
        }
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            if let Some(refused) = relay::refusal(access, status) {
                return Err(refused.into());
            }
            anyhow::bail!("{target}: {status}: {body}");
        }
        let res: Value = res.json().await?;
        let usage = &res["usage"];
        let n = |key: &str| usage[key].as_u64().unwrap_or(0);
        let tokens = Tokens {
            input: n("prompt_tokens"),
            cached: 0,
            output: n("completion_tokens"),
        };
        // OpenRouter は料金を答えに入れる
        match usage["cost"].as_f64() {
            Some(usd) => self.meter.billed(Use::Translate, tokens, usd),
            None => self.meter.tokens(Use::Translate, model, tokens),
        }
        res["choices"][0]["message"]["content"]
            .as_str()
            .map(|text| text.trim().to_owned())
            .context("no translation")
    }
}

/// PC の中の LLM の答えを縛る文法（llama.cpp の GBNF）。`⟦…⟧` の中は、その文の用語の名前だけ。
/// 用語のない文では、括弧を書けない。括弧の直後に字を続けられない（`⟦Sticky Bomb⟧s` のように語尾を付けさせない）
fn grammar<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let quoted: Vec<String> = names
        .map(|name| serde_json::to_string(name).expect("a string"))
        .collect();
    if quoted.is_empty() {
        return "root ::= [^⟦⟧]+\n".into();
    }
    format!(
        "root ::= text? (term after?)+\n\
         term ::= \"⟦\" ({}) \"⟧\"\n\
         after ::= [^⟦⟧a-zA-Zа-яёА-ЯЁ] [^⟦⟧]*\n\
         text ::= [^⟦⟧]+\n",
        quoted.join(" | ")
    )
}

/// Qwen-MT の言語コード。地域は分けない（中国語の簡体と繁体だけ別）
fn code(lang: &str) -> &str {
    match lang {
        "zh-Hans" => "zh",
        "zh-Hant" => "zh_tw",
        "no" => "nb",
        _ => lang::find(lang).map_or(lang, |l| l.stt),
    }
}

/// 用語を `⟦英語名⟧` に置き換えた文と、その文の用語（英語名と参照）。英語名のない用語は話した言葉のまま
fn tagged(spans: &[Span], glossary: Option<&Glossary>) -> (String, Vec<(String, String)>) {
    let mut source = String::new();
    let mut terms: Vec<(String, String)> = Vec::new();
    for span in spans {
        let named = match span {
            Span::Entity { reference, .. } => glossary
                .and_then(|g| g.entities.get(reference)?.names.get("en"))
                .map(|name| (name, reference)),
            Span::Text(_) => None,
        };
        match (span, named) {
            (_, Some((name, reference))) => {
                source += &format!("⟦{name}⟧");
                if !terms.iter().any(|(n, _)| n == name) {
                    terms.push((name.clone(), reference.clone()));
                }
            }
            (Span::Text(text), None) => source += text,
            (Span::Entity { spoken, .. }, None) => source += spoken,
        }
    }
    // 長い名前を先に探す（"Extra Spirit" の中の "Spirit" を別の用語にしない）
    terms.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    (source, terms)
}

/// 訳文の中の英語名を用語にする。名前を囲む括弧と引用符は外す
fn tokens(translation: &str, terms: &[(String, String)]) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut text = String::new();
    let mut rest = translation;
    while !rest.is_empty() {
        // 別の単語の一部（Rem と Remember）は名前にしない。大文字と小文字は区別しない（Chrome の翻訳の `findWord` と同じ）
        let joined = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
        let begins = !joined(text.chars().next_back());
        let term = terms.iter().filter(|_| begins).find(|(name, _)| {
            rest.get(..name.len())
                .is_some_and(|found| found.eq_ignore_ascii_case(name))
                && !joined(rest[name.len()..].chars().next())
        });
        let Some((name, reference)) = term else {
            let ch = rest.chars().next().expect("not empty");
            text.push(ch);
            rest = &rest[ch.len_utf8()..];
            continue;
        };
        rest = &rest[name.len()..];
        // 名前を囲んでいた括弧や引用符（⟦Haze⟧、«Haze»、"Haze"）を外す
        let closing = text
            .chars()
            .next_back()
            .and_then(|open| QUOTES.iter().find(|(o, _)| *o == open))
            .filter(|(_, close)| rest.starts_with(*close));
        if let Some((open, close)) = closing {
            text.truncate(text.len() - open.len_utf8());
            rest = &rest[close.len_utf8()..];
        }
        push_text(&mut tokens, &mut text);
        tokens.push(Token::Entity {
            reference: reference.clone(),
            spoken: None,
            ruby: None,
        });
    }
    push_text(&mut tokens, &mut text);
    tokens
}

/// 訳文から、アプリが用語として出せる名前（`names` のうち訳文で見つかったもの。出てきた順で、出た回数だけ）。
/// 評価（`examples/split_eval.rs`）が、字幕と同じ拾い方で数えるのに使う
pub fn shown_names(translation: &str, names: &[String]) -> Vec<String> {
    let mut terms: Vec<(String, String)> = names.iter().map(|n| (n.clone(), n.clone())).collect();
    terms.sort();
    terms.dedup();
    // 長い名前を先に探す（`tagged` と同じ）
    terms.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    tokens(translation, &terms)
        .into_iter()
        .filter_map(|token| match token {
            Token::Entity { reference, .. } => Some(reference),
            Token::Text { .. } => None,
        })
        .collect()
}

const QUOTES: [(char, char); 6] = [
    ('⟦', '⟧'),
    ('«', '»'),
    ('"', '"'),
    ('“', '”'),
    ('„', '“'),
    ('「', '」'),
];

/// 名前まで訳された用語の括弧（中は訳された名前）は、括弧だけ外して文字として出す
fn push_text(tokens: &mut Vec<Token>, text: &mut String) {
    let plain = std::mem::take(text).replace(['⟦', '⟧'], "");
    if !plain.is_empty() {
        tokens.push(Token::Text {
            text: plain,
            ruby: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glossary() -> Glossary {
        serde_json::from_value(json!({
            "clientVersion": 1, "languages": ["ja", "en"],
            "entities": {
                "hero:13": { "kind": "hero", "className": "h", "names": { "ja": "ヘイズ", "en": "Haze" } },
                "ability:4": { "kind": "ability", "className": "b", "names": { "ja": "バレットダンス", "en": "Bullet Dance" } },
                "item:9": { "kind": "item", "className": "i", "names": { "ja": "名前だけ" } }
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

    fn term(reference: &str) -> Token {
        Token::Entity {
            reference: reference.into(),
            spoken: None,
            ruby: None,
        }
    }

    fn terms() -> Vec<(String, String)> {
        vec![
            ("Bullet Dance".into(), "ability:4".into()),
            ("Haze".into(), "hero:13".into()),
        ]
    }

    #[test]
    fn terms_are_sent_as_english_names_in_brackets() {
        let spans = [
            entity("hero:13", "ヘイズ"),
            Span::Text("の".into()),
            entity("ability:4", "ウルト"),
            Span::Text("と".into()),
            entity("item:9", "名前だけ"),
            Span::Text("は強い。".into()),
        ];
        let (source, found) = tagged(&spans, Some(&glossary()));
        assert_eq!(source, "⟦Haze⟧の⟦Bullet Dance⟧と名前だけは強い。");
        assert_eq!(found, terms());
        // 辞書がなければ、話した言葉のまま訳す
        assert_eq!(tagged(&spans, None).0, "ヘイズのウルトと名前だけは強い。");
    }

    #[test]
    fn names_left_in_the_translation_become_terms() {
        assert_eq!(
            tokens("⟦Haze⟧'s ⟦Bullet Dance⟧ is strong.", &terms()),
            [
                term("hero:13"),
                text("'s "),
                term("ability:4"),
                text(" is strong.")
            ]
        );
    }

    #[test]
    fn names_are_found_without_brackets_or_in_quotes() {
        assert_eq!(
            tokens("«Bullet Dance» от Haze очень сильна.", &terms()),
            [
                term("ability:4"),
                text(" от "),
                term("hero:13"),
                text(" очень сильна.")
            ]
        );
    }

    #[test]
    fn names_are_found_in_any_letter_case() {
        assert_eq!(
            tokens("HAZE's bullet dance is strong.", &terms()),
            [
                term("hero:13"),
                text("'s "),
                term("ability:4"),
                text(" is strong.")
            ]
        );
    }

    #[test]
    fn shown_names_are_the_names_the_app_finds() {
        let names = ["Haze".to_owned(), "Sleep Dagger".to_owned()];
        assert_eq!(
            shown_names("haze's «Sleep Dagger», then ⟦Haze⟧ again. Hazel.", &names),
            ["Haze", "Sleep Dagger", "Haze"]
        );
    }

    #[test]
    fn a_name_inside_another_word_is_not_a_term() {
        let rem = vec![("Rem".to_owned(), "hero:1".to_owned())];
        assert_eq!(
            tokens("Remember, Rem's theorem.", &rem),
            [text("Remember, "), term("hero:1"), text("'s theorem.")]
        );
        // 括弧で囲んだ名前は、語尾が付いていても用語
        assert_eq!(
            tokens("ульта ⟦Rem⟧а", &rem),
            [text("ульта "), term("hero:1"), text("а")]
        );
    }

    #[test]
    fn a_translated_name_stays_as_text_without_brackets() {
        assert_eq!(
            tokens("⟦Хейз⟧ с ⟦Bullet Dance⟧.", &terms()),
            [text("Хейз с "), term("ability:4"), text(".")]
        );
    }

    /// 汎用の LLM には、指示文を付けて、推論をさせずに訳させる。翻訳専門のモデルには、今までの形で送る
    #[tokio::test]
    async fn a_general_model_gets_an_instruction_and_no_reasoning() {
        use crate::secrets::Provider;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let saw = seen.clone();
        let answer = move |axum::Json(body): axum::Json<Value>| {
            saw.lock().unwrap().push(body);
            async {
                axum::Json(json!({
                    "choices": [{ "message": { "content": "⟦Haze⟧ is strong." } }],
                    "usage": { "prompt_tokens": 90, "completion_tokens": 8 },
                }))
            }
        };
        let router = axum::Router::new()
            .route(
                "/qwen/chat/completions",
                axum::routing::post(answer.clone()),
            )
            .route(
                "/openai/chat/completions",
                axum::routing::post(answer.clone()),
            )
            .route("/openrouter/chat/completions", axum::routing::post(answer));
        tokio::spawn(async move { axum::serve(listener, router).await });
        let spans = [Span::Text("強い".into())];
        let http = reqwest::Client::new();
        for (provider, model) in [
            (Provider::Qwen, "qwen3.8-flash"),
            (Provider::OpenAi, "gpt-6-luna"),
            (Provider::Qwen, "qwen-mt-flash"),
            (Provider::OpenRouter, "google/gemma-4-26b-a4b-it"),
            (Provider::OpenRouter, "tencent/hy-mt2-30b-a3b"),
        ] {
            let meter = Arc::new(Meter::new(std::env::temp_dir().join("mt_test_usage.json")));
            let access = Access::at(provider, "code", &api);
            let translator = Translator::new(access, model, "ja".into(), vec!["ru".into()], meter);
            let translated = translator.translate(&http, &spans, None).await.unwrap();
            assert_eq!(translated["ru"], [text("Haze is strong.")], "{model}");
        }
        let seen = seen.lock().unwrap();
        let [qwen, luna, mt, gemma, hy] = &seen[..] else {
            panic!("{seen:?}");
        };
        let system = qwen["messages"][0]["content"].as_str().unwrap();
        assert!(
            system.starts_with("Translate the Japanese text into Russian."),
            "{system}"
        );
        assert!(system.contains("⟦ ⟧"));
        assert_eq!(
            qwen["messages"][1],
            json!({ "role": "user", "content": "強い" })
        );
        assert_eq!(qwen["enable_thinking"], false);
        assert_eq!(luna["reasoning_effort"], "none");
        assert_eq!(luna["messages"][0]["content"], system);
        assert_eq!(luna.get("enable_thinking"), None);
        // 翻訳専門のモデルは、指示文ではなく翻訳の設定で渡す
        assert_eq!(mt["messages"].as_array().unwrap().len(), 1);
        assert_eq!(mt["translation_options"]["target_lang"], "ru");
        // OpenRouter の汎用の LLM は、推論を止め、送った文を残さない会社だけに回させる
        assert_eq!(gemma["messages"][0]["content"], system);
        assert_eq!(gemma["reasoning"], json!({ "enabled": false }));
        assert_eq!(gemma["provider"], json!({ "data_collection": "deny" }));
        // OpenRouter の翻訳専門のモデル（Hy-MT2）は、モデルの説明にある指示文で送る
        assert_eq!(hy["messages"].as_array().unwrap().len(), 1);
        assert_eq!(hy.get("provider"), None);
    }

    /// 断られた鍵は字幕を止める理由にし、ほかの失敗はその言語だけ諦める
    #[tokio::test]
    async fn a_refused_key_stops_and_other_failures_skip_the_language() {
        use crate::secrets::Provider;
        use axum::http::StatusCode;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let router = axum::Router::new()
            .route(
                "/qwen/chat/completions",
                axum::routing::post(|| async { StatusCode::UNAUTHORIZED }),
            )
            .route(
                "/openai/chat/completions",
                axum::routing::post(|| async { StatusCode::BAD_GATEWAY }),
            );
        tokio::spawn(async move { axum::serve(listener, router).await });
        let translator = |provider| {
            let meter = Arc::new(Meter::new(std::env::temp_dir().join("mt_test_usage.json")));
            let access = Access::at(provider, "code", &api);
            Translator::new(
                access,
                "qwen-mt-flash",
                "ja".into(),
                vec!["en".into()],
                meter,
            )
        };
        let spans = [Span::Text("強い".into())];
        let http = reqwest::Client::new();
        let refused = translator(Provider::Qwen)
            .translate(&http, &spans, None)
            .await
            .unwrap_err();
        let provider = Provider::Qwen;
        assert_eq!(
            refused.downcast::<TranscriptionError>().unwrap(),
            TranscriptionError::InvalidKey { provider }
        );
        let failed = translator(Provider::OpenAi)
            .translate(&http, &spans, None)
            .await;
        assert!(failed.unwrap().is_empty());
    }

    /// サーバーには、文と翻訳先だけをまとめて 1 回で送る（モデルは送らない）。届いた翻訳先だけを出す。
    /// コードが断られたら字幕を止める理由にし、その仕事だけが止まっているなら、訳なしで続ける
    #[tokio::test]
    async fn the_server_translates_every_target_at_once() {
        use axum::Json;
        use axum::http::{HeaderMap, StatusCode};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}/v1", listener.local_addr().unwrap());
        let translate = |headers: HeaderMap, Json(body): Json<Value>| async move {
            let refuse = |error: &str| (StatusCode::FORBIDDEN, Json(json!({ "error": error })));
            match headers["authorization"].to_str().unwrap() {
                "Bearer good" => {
                    let asked =
                        json!({ "text": "⟦Haze⟧強い", "source": "ja", "targets": ["en", "ru"] });
                    assert_eq!(body, asked);
                    let translations = json!({ "en": "⟦Haze⟧ is strong" });
                    (
                        StatusCode::OK,
                        Json(json!({ "translations": translations })),
                    )
                }
                "Bearer limit" => refuse("limit"),
                _ => refuse("stopped"),
            }
        };
        let router = axum::Router::new().route("/v2/translate", axum::routing::post(translate));
        tokio::spawn(async move { axum::serve(listener, router).await });
        let translated = |code: &str| {
            let meter = Arc::new(Meter::new(std::env::temp_dir().join("mt_test_usage.json")));
            let server = Server::new(&api, code.into());
            let targets = vec!["en".into(), "ru".into()];
            let translator = Translator::server(server, "ja".into(), targets, meter);
            let glossary = glossary();
            async move {
                let spans = [entity("hero:13", "ヘイズ"), Span::Text("強い".into())];
                let http = reqwest::Client::new();
                translator.translate(&http, &spans, Some(&glossary)).await
            }
        };
        let good = translated("good").await.unwrap();
        assert_eq!(
            good,
            Translations::from([("en".into(), vec![term("hero:13"), text(" is strong")])])
        );
        let refused = translated("limit").await.unwrap_err();
        assert_eq!(
            refused.downcast::<TranscriptionError>().unwrap(),
            TranscriptionError::BetaLimit
        );
        assert!(translated("stopped").await.unwrap().is_empty());
    }

    #[test]
    fn the_grammar_only_lets_the_terms_of_the_sentence_in_brackets() {
        assert_eq!(grammar([].into_iter()), "root ::= [^⟦⟧]+\n");
        let with_terms = grammar(["Haze", "Smoke \"Bomb\""].into_iter());
        assert!(with_terms.contains(r#"term ::= "⟦" ("Haze" | "Smoke \"Bomb\"") "⟧""#));
        assert!(with_terms.starts_with("root ::= text? (term after?)+\n"));
    }

    /// 実際に llama.cpp を立てて訳す（初回は約 5 GB を取る）。`cargo test -- --ignored local_llm`
    #[tokio::test]
    #[ignore = "downloads llama.cpp and a 5 GB model"]
    async fn local_llm_keeps_the_terms() {
        let dir = std::env::var("LLAMA_TEST_DIR").expect("LLAMA_TEST_DIR");
        let llama = Arc::new(
            Llama::start(std::path::Path::new(&dir), None, &|_, _| {})
                .await
                .unwrap(),
        );
        let meter = Arc::new(Meter::new(std::env::temp_dir().join("mt_local_usage.json")));
        let translator =
            Translator::local(llama, "ja".into(), vec!["en".into(), "ru".into()], meter);
        let (source, terms) = (
            "⟦Haze⟧来てる来てる、⟦Smoke Bomb⟧で隠れてるから気をつけて。",
            vec![
                ("Smoke Bomb".to_owned(), "b".to_owned()),
                ("Haze".to_owned(), "a".to_owned()),
            ],
        );
        for target in ["en", "ru"] {
            let Backend::Local(llama) = &translator.backend else {
                unreachable!()
            };
            let text = translator
                .ask_local(llama, source, target, &terms)
                .await
                .unwrap();
            println!("{target}: {text}");
            assert!(
                text.contains("⟦Haze⟧") && text.contains("⟦Smoke Bomb⟧"),
                "{text}"
            );
        }
    }

    #[test]
    fn language_codes_follow_the_model() {
        assert_eq!(code("ja"), "ja");
        assert_eq!(code("zh-Hans"), "zh");
        assert_eq!(code("zh-Hant"), "zh_tw");
        assert_eq!(code("pt-BR"), "pt");
        assert_eq!(code("no"), "nb");
    }
}
