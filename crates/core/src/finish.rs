//! 確定した文に、工程ごとの選択（`Engines`）どおりに用語・ふりがな・翻訳を付ける（DESIGN.md 9.2、9.6）。
//!
//! 用語の区切りは LLM か辞書の一致か Decisions API、ふりがなは LLM か形態素解析、翻訳は LLM か Chrome の翻訳か
//! 翻訳専門のモデルで作り、文字の位置で 1 つの字幕にまとめる（`enrich::source_tokens`）。確定の文を Chrome と
//! OpenAI のどちらが作っても、この流れを通る。ふりがなは、用語と翻訳と同時に聞く。
//! 用語と翻訳は、LLM なら 1 回で聞き、ほかは用語を決めてから訳す（訳文の名前を用語から作る。DESIGN.md 9.7）

use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{broadcast, mpsc};

use crate::access::{Access, Accesses};
use crate::annotate::mark_first_person;
use crate::chrome;
use crate::decide::{Asker, Decider};
use crate::enrich::{Enricher, Span, cover, source_tokens};
use crate::furigana;
use crate::llama::Llama;
use crate::mt::Translator;
use crate::people;
use crate::reading::Furigana;
use crate::secrets::Provider;
use crate::settings::{
    EmphasisSource, Phase, RubySource, Settings, TermsEngine, TranslationEngine, TranslationSource,
};
use crate::store::{ChromeEvent, Store};
use crate::types::{
    Enriched, Glossary, RubySegment, TermName, Token, TranscriptionError, Translatable,
    Translations,
};

pub struct Finisher {
    store: Arc<Store>,
    /// LLM に聞く工程があり、鍵があるときだけ
    enricher: Option<Enricher>,
    /// 用語を Decisions API で確かめるときだけ
    decider: Option<Decider>,
    /// 用語の候補を出す（Chrome の中の Gemini Nano に確かめさせるとき）
    asker: Asker,
    /// 翻訳専門のモデルで訳すときだけ
    translator: Option<Translator>,
    /// ふりがなを判定モデルに選ばせるときだけ
    chooser: Option<Chooser>,
}

/// ふりがなの読みを、辞書の候補から選ぶ判定モデル
struct Chooser {
    access: Access,
    model: &'static str,
    deadlock: bool,
}

/// 字幕の処理に返すもの
pub enum Finished {
    /// 書きかけの訳文（LLM の訳だけ）
    Draft {
        id: String,
        translations: Translations,
    },
    /// 翻訳に失敗しても、用語とふりがなは付いている
    Done { id: String, enriched: Enriched },
    /// LLM に鍵やギフトコードが断られた。送り直しても断られるだけなので、字幕を止める
    Refused(TranscriptionError),
}

/// 確定した文と、PC の中で付けた読みと用語（`Annotator::annotate` に確定の `Phase` を渡したもの）
pub struct Heard {
    pub id: String,
    pub text: String,
    /// 直前の確定字幕（古い順）。LLM の文脈
    pub context: Vec<String>,
    pub tokens: Vec<Token>,
}

impl Finisher {
    /// `accesses` は工程が使う会社の送り先と鍵。`llama` は PC の中の LLM（使う工程があるときだけ立ててある）
    pub fn new(
        store: Arc<Store>,
        settings: &Settings,
        accesses: &Accesses,
        llama: Option<Arc<Llama>>,
    ) -> Self {
        let lang = &settings.mic_language;
        let engines = settings.engines;
        let local = |engine: bool| llama.clone().filter(|_| engine);
        let decider = match local(engines.terms == TermsEngine::Gemma) {
            Some(llama) => Some(Decider::local(llama, lang.clone(), store.meter())),
            None => engines.judge().and_then(|judge| {
                let access = accesses.get(judge.provider)?;
                Some(Decider::new(
                    access,
                    judge.name,
                    lang.clone(),
                    store.meter(),
                ))
            }),
        };
        let targets = settings.translate_to.clone();
        let translator = match local(engines.translation == TranslationEngine::Gemma) {
            Some(llama) => Some(Translator::local(
                llama,
                lang.clone(),
                targets,
                store.meter(),
            )),
            None => engines.translation.mt().and_then(|mt| {
                Some(Translator::new(
                    accesses.get(mt.provider)?,
                    mt.name,
                    lang.clone(),
                    targets,
                    store.meter(),
                ))
            }),
        };
        let enricher = accesses.get(Provider::OpenAi).map(|access| {
            Enricher::new(
                access,
                &settings.models_in_use(),
                settings.mic_language.clone(),
                settings.translate_to.clone(),
                settings.deadlock_mode,
                store.meter(),
            )
        });
        let chooser = settings.engines.furigana.judge().and_then(|judge| {
            Some(Chooser {
                access: accesses.get(judge.provider)?,
                model: judge.name,
                deadlock: settings.deadlock_mode,
            })
        });
        Self {
            store,
            enricher,
            decider,
            asker: Asker::new(lang.clone()),
            translator,
            chooser,
        }
    }

    /// `translate` は Chrome の翻訳に渡す文。`phase` は確定の作り方。
    /// 字幕を止めずに表示を切り替えられるので、文ごとにそのときの設定を渡す
    pub fn spawn(
        self: &Arc<Self>,
        heard: Heard,
        translate: Translatable,
        phase: Phase,
        done: mpsc::Sender<Finished>,
    ) {
        let finisher = self.clone();
        // 用語が PC の中の一致で決まっているなら、すぐページに頼む（頼んだときから聞くので、訳がすぐ届いても
        // 取りこぼさない）。Decisions で確かめるなら、用語が決まってから頼む
        let decides = matches!(
            phase.emphasis,
            Some(EmphasisSource::Decisions | EmphasisSource::Chrome)
        );
        let chrome = (phase.translation == Some(TranslationSource::ChromeTranslator) && !decides)
            .then(|| {
                chrome::ask_final_translation(&self.store, heard.id.clone(), translate.clone())
            })
            .flatten();
        tokio::spawn(async move {
            let started = Instant::now();
            let id = heard.id.clone();
            let on_draft = |translations| {
                // 書きかけは次のものが追い越すので、詰まっていたら捨てる
                let _ = done.try_send(Finished::Draft {
                    id: id.clone(),
                    translations,
                });
            };
            let asked = finisher.finish(heard, translate, chrome, &phase, on_draft);
            let finished = match asked.await {
                Ok(enriched) => Finished::Done { id, enriched },
                Err(refused) => Finished::Refused(refused),
            };
            tracing::debug!("finished in {:?}", started.elapsed());
            let _ = done.send(finished).await;
        });
    }

    /// `translate` は、PC の中で付けた用語で作った、Chrome の翻訳に渡す文。`chrome` は、もうページに頼んだ訳
    async fn finish(
        &self,
        heard: Heard,
        translate: Translatable,
        chrome: Option<broadcast::Receiver<ChromeEvent>>,
        phase: &Phase,
        on_draft: impl FnMut(Translations),
    ) -> Result<Enriched, TranscriptionError> {
        let Heard {
            id,
            text,
            context,
            tokens,
        } = heard;
        let llm = self.enricher.as_ref();
        let llm_words = llm.filter(|_| {
            phase.translation == Some(TranslationSource::Llm)
                || phase.emphasis == Some(EmphasisSource::Llm)
        });
        // 辞書は文ごとに store から取る（後から届いたり、作り直されたりする）
        let loaded = self.store.glossary();
        let glossary = loaded.as_ref().map(|l| l.glossary.clone());
        let local = spans_of(&tokens);
        let (ruby, words) = tokio::join!(
            async {
                // 読みの辞書は、判定モデルに選ばせるときだけ取る（日本語でなければ、広げない）
                let chooser = self.chooser.as_ref().zip(
                    (phase.ruby == Some(RubySource::Decider))
                        .then(Furigana::shared)
                        .flatten(),
                );
                let asked = match (phase.ruby, llm, chooser) {
                    (Some(source), Some(e), _) if source.llm() => e.furigana(&text, &context).await,
                    (_, _, Some((chooser, dictionary))) => {
                        let meter = self.store.meter();
                        let judge = furigana::Judge {
                            deadlock: chooser.deadlock,
                            access: &chooser.access,
                            model: chooser.model,
                            meter: &meter,
                        };
                        furigana::choose(crate::http(), &judge, dictionary, &text, &context)
                            .await
                            .map(Some)
                    }
                    _ => Ok(None),
                };
                answer("furigana", asked)
            },
            async {
                if let Some(e) = llm_words {
                    let asked = e
                        .terms_and_translations(&text, &context, glossary.as_deref(), on_draft)
                        .await;
                    return answer("terms and translation", asked);
                }
                // 用語を決めてから訳す。確かめられなければ、PC の中で付けた用語のまま訳す
                let decider = self
                    .decider
                    .as_ref()
                    .filter(|_| phase.emphasis == Some(EmphasisSource::Decisions));
                let nano = phase.emphasis == Some(EmphasisSource::Chrome);
                // 配信者が今使っているヒーロー（「ウルト」だけ言ったときに、そのスキルを候補にする）
                let playing = self.store.playing();
                let playing = playing.as_deref();
                let decided = match (decider, &loaded) {
                    (Some(d), Some(loaded)) => answer(
                        "terms",
                        d.terms(crate::http(), &text, &context, playing, loaded)
                            .await
                            .map(Some),
                    )?,
                    // 候補がなければ聞かない。ページが答えなければ、PC の中で付けた用語のまま
                    (None, Some(loaded)) if nano => {
                        match self.asker.ask("", &text, &context, playing, loaded) {
                            Some(asked) => chrome::decide(&self.store, &asked, &id)
                                .await
                                .map(|choices| asked.spans(|name| choices.get(name).cloned())),
                            None => None,
                        }
                    }
                    _ => None,
                };
                let translations = match phase.translation {
                    Some(TranslationSource::ChromeTranslator) => {
                        let events = chrome.or_else(|| {
                            let translate = match &decided {
                                Some(spans) => translatable(spans, glossary.as_deref()),
                                None => translate,
                            };
                            chrome::ask_final_translation(&self.store, id.clone(), translate)
                        });
                        match events {
                            Some(events) => chrome::final_translation(events, &id).await,
                            None => Translations::new(),
                        }
                    }
                    Some(TranslationSource::Mt) => match &self.translator {
                        Some(t) => {
                            let spans = decided.as_deref().unwrap_or(&local);
                            let asked = t.translate(crate::http(), spans, glossary.as_deref());
                            answer("translation", asked.await.map(Some))?.unwrap_or_default()
                        }
                        None => Translations::new(),
                    },
                    Some(TranslationSource::Llm) | None => Translations::new(),
                };
                Ok(Some((
                    decided.unwrap_or_else(|| local.clone()),
                    translations,
                )))
            },
        );
        // LLM に聞かなかったものと、答えが通らなかったものは、PC の中で付けたものを使う
        let ruby = match ruby? {
            Some(sentence) => with_term_readings(&sentence, &tokens),
            None => ruby_of(&tokens),
        };
        let (spans, translations) = words?.unwrap_or_else(|| (local, Translations::new()));
        let source = source_tokens(&spans, &ruby);
        // 用語を強調するなら、自分を指す言葉（一人称）にもアイコンを付ける。用語と翻訳が決まったあとなので、
        // 翻訳では名前に置き換わらない
        let me = people::first_person(&self.store.settings().people);
        let furigana = (phase.emphasis.is_some()).then(Furigana::shared).flatten();
        let source = match (furigana, me) {
            (Some(furigana), Some(me)) => mark_first_person(furigana, &text, source, &me),
            _ => source,
        };
        Ok(Enriched {
            source,
            translations,
        })
    }
}

/// Chrome の翻訳に渡す文。用語を英語の名前に置き換える（`annotate` が PC の中の用語で作るものと同じ形）
fn translatable(spans: &[Span], glossary: Option<&Glossary>) -> Translatable {
    let mut translate = Translatable::default();
    for span in spans {
        let named = match span {
            Span::Entity { reference, .. } => glossary
                .and_then(|g| g.entities.get(reference)?.names.get("en"))
                .map(|en| (reference, en)),
            Span::Text(_) => None,
        };
        match (span, named) {
            (_, Some((reference, en))) => {
                translate.text += en;
                translate.terms.push(TermName {
                    reference: reference.clone(),
                    en: en.clone(),
                });
            }
            (Span::Text(text), None) => translate.text += text,
            (Span::Entity { spoken, .. }, None) => translate.text += spoken,
        }
    }
    translate
}

/// LLM の答え。断られたら字幕を止める理由にし、ほかの失敗（回線、タイムアウト、読めない答え）はその文だけ諦める
fn answer<T>(
    what: &str,
    result: anyhow::Result<Option<T>>,
) -> Result<Option<T>, TranscriptionError> {
    result.or_else(|e| match e.downcast::<TranscriptionError>() {
        Ok(refused) => Err(refused),
        Err(e) => {
            tracing::warn!("{what}: {e:#}");
            Ok(None)
        }
    })
}

/// 用語の区切り。ふつうの文字はつなげる
fn spans_of(tokens: &[Token]) -> Vec<Span> {
    let mut spans = Vec::new();
    for token in tokens {
        match (token, spans.last_mut()) {
            (Token::Text { text, .. }, Some(Span::Text(t))) => t.push_str(text),
            (Token::Text { text, .. }, _) => spans.push(Span::Text(text.clone())),
            (
                Token::Entity {
                    reference, spoken, ..
                },
                _,
            ) => spans.push(Span::Entity {
                reference: reference.clone(),
                spoken: spoken.clone().unwrap_or_default(),
            }),
        }
    }
    spans
}

/// 判定モデルや LLM が直した文のふりがなのうち、PC の中で見つけた用語のところは、用語の読みに戻す。
/// 用語の読みは、ゲームの辞書の名前の読み（確かめてある）。文を読みの辞書で読むと、固有の名前を読み誤る
fn with_term_readings(sentence: &[RubySegment], tokens: &[Token]) -> Vec<RubySegment> {
    let mut placed = Vec::new();
    let mut pos = 0;
    for seg in sentence {
        let len = seg.text.chars().count();
        placed.push((pos, pos + len, seg));
        pos += len;
    }
    let mut start = 0;
    let mut out = Vec::new();
    for token in tokens {
        let text = match token {
            Token::Text { text, .. } => text.as_str(),
            Token::Entity { spoken, .. } => spoken.as_deref().unwrap_or_default(),
        };
        match token {
            Token::Entity {
                ruby: Some(ruby), ..
            } => out.extend(ruby.iter().cloned()),
            _ => out.extend(cover(text, start, &placed)),
        }
        start += text.chars().count();
    }
    out
}

/// 文全体のふりがな（つなげると原文になる）
fn ruby_of(tokens: &[Token]) -> Vec<RubySegment> {
    tokens
        .iter()
        .flat_map(|token| match token {
            Token::Text { text, ruby } => vec![RubySegment {
                text: text.clone(),
                ruby: ruby.clone(),
            }],
            Token::Entity {
                ruby: Some(ruby), ..
            } => ruby.clone(),
            Token::Entity { spoken, .. } => vec![RubySegment {
                text: spoken.clone().unwrap_or_default(),
                ruby: None,
            }],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(text: &str, ruby: Option<&str>) -> Token {
        Token::Text {
            text: text.into(),
            ruby: ruby.map(Into::into),
        }
    }

    fn seg(text: &str, ruby: Option<&str>) -> RubySegment {
        RubySegment {
            text: text.into(),
            ruby: ruby.map(Into::into),
        }
    }

    /// 「拡張(かくちょう)マガジンを取(と)る」の、用語とふりがなを PC の中で付けたもの
    fn local() -> Vec<Token> {
        vec![
            Token::Entity {
                reference: "item:1".into(),
                spoken: Some("拡張マガジン".into()),
                ruby: Some(vec![seg("拡張", Some("かくちょう")), seg("マガジン", None)]),
            },
            text("を", None),
            text("取", Some("と")),
            text("る", None),
        ]
    }

    #[test]
    fn local_tokens_split_into_terms_and_furigana() {
        assert_eq!(
            spans_of(&local()),
            [
                Span::Entity {
                    reference: "item:1".into(),
                    spoken: "拡張マガジン".into(),
                },
                Span::Text("を取る".into()),
            ]
        );
        assert_eq!(
            ruby_of(&local()),
            [
                seg("拡張", Some("かくちょう")),
                seg("マガジン", None),
                seg("を", None),
                seg("取", Some("と")),
                seg("る", None),
            ]
        );
    }

    #[test]
    fn local_terms_and_furigana_come_back_together() {
        let tokens = source_tokens(&spans_of(&local()), &ruby_of(&local()));
        assert_eq!(tokens[0], local()[0]);
        assert_eq!(
            tokens[1..],
            [text("を", None), text("取", Some("と")), text("る", None)]
        );
    }

    #[test]
    fn llm_furigana_lands_on_dictionary_terms() {
        let llm = [
            seg("拡張", Some("かくちょう")),
            seg("マガジンを", None),
            seg("取", Some("と")),
            seg("る", None),
        ];
        let tokens = source_tokens(&spans_of(&local()), &llm);
        assert!(
            matches!(&tokens[0], Token::Entity { ruby: Some(r), .. } if r[0] == seg("拡張", Some("かくちょう")))
        );
    }

    /// 判定モデルや LLM が直した文の読みでも、用語のところは、PC の中で付けた用語の読みのまま
    #[test]
    fn term_readings_survive_a_corrected_sentence() {
        // 文を読みの辞書で読むと「かくばり」になる名前（用語の読みは「かくちょう」）
        let sentence = [
            seg("拡張", Some("かくばり")),
            seg("マガジンは", None),
            seg("辛", Some("から")),
            seg("い", None),
        ];
        let tokens = [
            Token::Entity {
                reference: "item:1".into(),
                spoken: Some("拡張マガジン".into()),
                ruby: Some(vec![seg("拡張", Some("かくちょう")), seg("マガジン", None)]),
            },
            text("は", None),
            text("辛", Some("つら")),
            text("い", None),
        ];
        assert_eq!(
            with_term_readings(&sentence, &tokens),
            [
                seg("拡張", Some("かくちょう")),
                seg("マガジン", None),
                seg("は", None),
                seg("辛", Some("から")),
                seg("い", None),
            ]
        );
    }

    #[test]
    fn a_refusal_stops_and_other_failures_skip_the_sentence() {
        let refused: anyhow::Result<Option<()>> = Err(TranscriptionError::GiftRevoked.into());
        assert_eq!(
            answer("x", refused).unwrap_err(),
            TranscriptionError::GiftRevoked
        );
        let offline: anyhow::Result<Option<()>> = Err(anyhow::anyhow!("offline"));
        assert_eq!(answer("x", offline).unwrap(), None);
    }

    /// 頼んだときから聞くので、訳がすぐ届いても取りこぼさない
    #[tokio::test]
    async fn the_chrome_translation_is_put_on_the_subtitle() {
        let store = Store::new(Ok(Settings::default()), std::path::PathBuf::new());
        let _page = store.subscribe_chrome_commands();
        let translate = Translatable {
            text: "Extended Magazineを取る".into(),
            terms: Vec::new(),
        };
        let events = chrome::ask_final_translation(&store, "1".into(), translate.clone()).unwrap();
        let translations: Translations = [("en".to_owned(), vec![text("Take it", None)])].into();
        store.send_chrome_event(ChromeEvent::Translated(crate::types::Translated {
            id: "1".into(),
            is_final: true,
            translations: translations.clone(),
        }));
        let finisher = Finisher::new(store, &Settings::default(), &Accesses::default(), None);
        let phase = Settings::default().plan().finals;
        let heard = Heard {
            id: "1".into(),
            text: "拡張マガジンを取る".into(),
            context: Vec::new(),
            tokens: local(),
        };
        let enriched = finisher
            .finish(heard, translate, Some(events), &phase, |_| {})
            .await
            .unwrap();
        assert_eq!(enriched.translations, translations);
        assert_eq!(enriched.source[0], local()[0]);
    }

    #[test]
    fn decided_terms_are_sent_to_chrome_as_english_names() {
        let glossary: Glossary = serde_json::from_value(serde_json::json!({
            "clientVersion": 1, "languages": ["ja", "en"],
            "entities": {
                "item:1": { "kind": "item", "className": "i", "names": { "ja": "基礎拡張マガジン", "en": "Extended Magazine" } }
            }
        }))
        .unwrap();
        let spans = [
            Span::Entity {
                reference: "item:1".into(),
                spoken: "拡マガ".into(),
            },
            Span::Text("を取る".into()),
        ];
        assert_eq!(
            translatable(&spans, Some(&glossary)),
            Translatable {
                text: "Extended Magazineを取る".into(),
                terms: vec![TermName {
                    reference: "item:1".into(),
                    en: "Extended Magazine".into(),
                }],
            }
        );
    }

    /// 偽の Worker。Decisions は「拡マガ」を item:1 と答え（`decides` が false なら 404）、Qwen は英語名をそのまま返す
    async fn split_server(decides: bool) -> String {
        use axum::Json;
        use axum::http::StatusCode;
        use serde_json::{Value, json};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let decisions = move |Json(body): Json<Value>| async move {
            if !decides {
                return Err(StatusCode::NOT_FOUND);
            }
            let asked = body["questions"][0]["instructions"].as_str().unwrap();
            assert!(asked.contains("拡マガ"), "{asked}");
            Ok(Json(json!({
                "answers": [{ "type": "choice", "name": "c0", "choice": "item:1" }],
                "usage": { "input_tokens": 100, "output_tokens": 0 },
            })))
        };
        let translate = |Json(body): Json<Value>| async move {
            let source = body["messages"][0]["content"].as_str().unwrap();
            let text = source.replace("を取る", " first");
            Json(json!({
                "choices": [{ "message": { "content": text } }],
                "usage": { "prompt_tokens": 10, "completion_tokens": 5 },
            }))
        };
        // Jev の形の口。問いは名前ごとで、選択肢は `criteria`
        let systemone = |Json(body): Json<Value>| async move {
            assert_eq!(body["model"], "jev-latest");
            assert!(body["questions"]["c0"]["criteria"]["item:1"].is_string());
            Json(json!({
                "answers": { "c0": { "choice": "item:1" } },
                "usage": { "input_tokens": 100, "output_tokens": 0 },
            }))
        };
        // Hy-MT2 は指示文の最後に原文がある。OpenRouter は料金を答えに入れる
        let hy_mt = |Json(body): Json<Value>| async move {
            let prompt = body["messages"][0]["content"].as_str().unwrap();
            assert!(prompt.contains("into English"), "{prompt}");
            let source = prompt.rsplit("[Source Text]\n").next().unwrap();
            Json(json!({
                "choices": [{ "message": { "content": source.replace("を取る", " first") } }],
                "usage": { "prompt_tokens": 10, "completion_tokens": 5, "cost": 0.001 },
            }))
        };
        let router = axum::Router::new()
            .route("/openai/decisions", axum::routing::post(decisions))
            .route("/qwen/chat/completions", axum::routing::post(translate))
            .route("/typesafe/systemone", axum::routing::post(systemone))
            .route("/openrouter/chat/completions", axum::routing::post(hy_mt));
        tokio::spawn(async move { axum::serve(listener, router).await });
        api
    }

    /// 用語を Decisions で決めてから、その用語で訳す。「拡マガ」は辞書の名前と一致しないので、PC の中では付かない
    async fn finished_split(decides: bool) -> Enriched {
        use crate::settings::{TermsEngine, TranslationEngine};
        finished_with(
            decides,
            TermsEngine::Decisions,
            TranslationEngine::QwenMtFlash,
        )
        .await
        .0
    }

    /// 仕上げた文と、用語と翻訳にかかった額
    async fn finished_with(
        decides: bool,
        terms: crate::settings::TermsEngine,
        translation: crate::settings::TranslationEngine,
    ) -> (Enriched, f64) {
        use crate::settings::Engines;
        let api = split_server(decides).await;
        let settings = Settings {
            engines: Engines {
                terms,
                translation,
                ..Engines::CHROME
            },
            translate_to: vec!["en".into()],
            ..Default::default()
        };
        let store = Store::new(Ok(settings.clone()), std::path::PathBuf::new());
        let glossary: Glossary = serde_json::from_value(serde_json::json!({
            "clientVersion": 1, "languages": ["ja", "en"],
            "entities": {
                "item:1": { "kind": "item", "className": "i", "names": { "ja": "基礎拡張マガジン", "en": "Extended Magazine" } }
            }
        }))
        .unwrap();
        store.set_glossary(crate::assets::Loaded {
            glossary: Arc::new(glossary),
            requested: Vec::new(),
            root: std::path::PathBuf::new(),
            aliases: Arc::new([("item:1".to_owned(), vec!["拡マガ".to_owned()])].into()),
        });
        let gift = |provider| crate::access::Access::gift(provider, "code".into(), &api);
        let accesses = Accesses::new(settings.engines.providers().into_iter().map(gift));
        let finisher = Finisher::new(store.clone(), &settings, &accesses, None);
        let heard = Heard {
            id: "1".into(),
            text: "拡マガを取る".into(),
            context: Vec::new(),
            tokens: vec![text("拡マガを取る", None)],
        };
        let phase = settings.plan().finals;
        let finished = finisher.finish(heard, Translatable::default(), None, &phase, |_| {});
        let enriched = finished.await.unwrap();
        let usage = store.meter().watch().borrow().session.clone();
        (enriched, usage[&crate::usage::Use::Translate].usd)
    }

    /// 用語は Jev、翻訳は OpenRouter の Hy-MT2。どちらも口の形が違うだけで、結果は同じ
    #[tokio::test]
    async fn other_judges_and_translators_give_the_same_subtitle() {
        use crate::settings::{TermsEngine, TranslationEngine};
        let (enriched, usd) = finished_with(true, TermsEngine::Jev, TranslationEngine::HyMt2).await;
        assert_eq!(enriched, finished_split(true).await);
        // Jev は入力 100 トークン（100 万トークン $0.042）、Hy-MT2 は答えに入っていた料金
        assert!((usd - (100.0 * 0.042 / 1e6 + 0.001)).abs() < 1e-9, "{usd}");
    }

    #[tokio::test]
    async fn decided_terms_mark_the_source_and_the_translation() {
        let enriched = finished_split(true).await;
        let term = |spoken: Option<&str>| Token::Entity {
            reference: "item:1".into(),
            spoken: spoken.map(Into::into),
            ruby: None,
        };
        assert_eq!(
            enriched.source,
            [term(Some("拡マガ")), text("を取る", None)]
        );
        assert_eq!(
            enriched.translations["en"],
            [term(None), text(" first", None)]
        );
    }

    #[tokio::test]
    async fn without_an_answer_the_local_terms_are_translated() {
        // 古いサーバーには Decisions の口がない。PC の中で付けた用語（ここではなし）のまま訳す
        let enriched = finished_split(false).await;
        assert_eq!(enriched.source, [text("拡マガを取る", None)]);
        assert_eq!(enriched.translations["en"], [text("拡マガ first", None)]);
    }

    #[tokio::test]
    async fn without_llm_the_local_marks_are_kept() {
        let store = Store::new(Ok(Settings::default()), std::path::PathBuf::new());
        let finisher = Finisher::new(store, &Settings::default(), &Accesses::default(), None);
        let phase = Settings::default().plan().finals;
        let heard = Heard {
            id: "1".into(),
            text: "拡張マガジンを取る".into(),
            context: Vec::new(),
            tokens: local(),
        };
        // ページが開いていないので、訳は付かない
        let enriched = finisher
            .finish(heard, Translatable::default(), None, &phase, |_| {})
            .await
            .unwrap();
        assert!(enriched.translations.is_empty());
        assert_eq!(enriched.source[0], local()[0]);
    }
}
