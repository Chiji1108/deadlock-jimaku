//! 確定した文に、工程ごとの選択（`Engines`）どおりに用語・ふりがな・翻訳を付ける（DESIGN.md 9.2、9.6）。
//!
//! 用語の区切りは LLM か辞書の一致、ふりがなは LLM か形態素解析、翻訳は LLM か Chrome の翻訳で作り、
//! 文字の位置で 1 つの字幕にまとめる（`enrich::source_tokens`）。確定の文を Chrome と OpenAI の
//! どちらが作っても、この流れを通る。聞くものは同時に投げる

use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{broadcast, mpsc};

use crate::chrome;
use crate::enrich::{Enricher, Span, source_tokens};
use crate::openai::Access;
use crate::settings::{EmphasisSource, Phase, RubySource, Settings, TranslationSource};
use crate::store::{ChromeEvent, Store};
use crate::types::{Enriched, RubySegment, Token, TranscriptionError, Translatable, Translations};

pub struct Finisher {
    store: Arc<Store>,
    /// LLM に聞く工程があり、鍵があるときだけ
    enricher: Option<Enricher>,
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
    pub fn new(store: Arc<Store>, settings: &Settings, access: Option<Access>) -> Self {
        let enricher = access.map(|access| {
            Enricher::new(
                access,
                &settings.models,
                settings.mic_language.clone(),
                settings.translate_to.clone(),
                settings.deadlock_mode,
                store.meter(),
            )
        });
        Self { store, enricher }
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
        let chrome = (phase.translation == Some(TranslationSource::ChromeTranslator))
            .then(|| chrome::ask_final_translation(&self.store, heard.id.clone(), translate))
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
            let finished = match finisher.finish(heard, &phase, chrome, on_draft).await {
                Ok(enriched) => Finished::Done { id, enriched },
                Err(refused) => Finished::Refused(refused),
            };
            tracing::debug!("finished in {:?}", started.elapsed());
            let _ = done.send(finished).await;
        });
    }

    /// `chrome` は Chrome のページに頼んだ訳
    async fn finish(
        &self,
        heard: Heard,
        phase: &Phase,
        chrome: Option<broadcast::Receiver<ChromeEvent>>,
        on_draft: impl FnMut(Translations),
    ) -> Result<Enriched, TranscriptionError> {
        let Heard {
            id,
            text,
            context,
            tokens,
        } = heard;
        let llm = self.enricher.as_ref();
        let llm_ruby = llm.filter(|_| phase.ruby == Some(RubySource::Llm));
        let llm_words = llm.filter(|_| {
            phase.translation == Some(TranslationSource::Llm)
                || phase.emphasis == Some(EmphasisSource::Llm)
        });
        // 辞書は文ごとに store から取る（後から届いたり、作り直されたりする）
        let glossary = self.store.glossary().map(|l| l.glossary);
        let (ruby, words, translated) = tokio::join!(
            async {
                match llm_ruby {
                    Some(e) => answer("furigana", e.furigana(&text, &context).await)
                        .map(|ruby| Some(ruby.unwrap_or_default())),
                    None => Ok(None),
                }
            },
            async {
                match llm_words {
                    Some(e) => answer(
                        "terms and translation",
                        e.terms_and_translations(&text, &context, glossary.as_deref(), on_draft)
                            .await,
                    ),
                    None => Ok(None),
                }
            },
            async {
                match chrome {
                    Some(events) => chrome::final_translation(events, &id).await,
                    None => Translations::new(),
                }
            },
        );
        // LLM に聞かなかったものと、答えが通らなかったものは、PC の中で付けたものを使う
        let ruby = ruby?.unwrap_or_else(|| ruby_of(&tokens));
        let (spans, translations) = words?.unwrap_or_else(|| (spans_of(&tokens), translated));
        Ok(Enriched {
            source: source_tokens(&spans, &ruby),
            translations,
        })
    }
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
        let events = chrome::ask_final_translation(&store, "1".into(), translate).unwrap();
        let translations: Translations = [("en".to_owned(), vec![text("Take it", None)])].into();
        store.send_chrome_event(ChromeEvent::Translated(crate::types::Translated {
            id: "1".into(),
            is_final: true,
            translations: translations.clone(),
        }));
        let finisher = Finisher::new(store, &Settings::default(), None);
        let phase = Settings::default().plan().finals;
        let heard = Heard {
            id: "1".into(),
            text: "拡張マガジンを取る".into(),
            context: Vec::new(),
            tokens: local(),
        };
        let enriched = finisher
            .finish(heard, &phase, Some(events), |_| {})
            .await
            .unwrap();
        assert_eq!(enriched.translations, translations);
        assert_eq!(enriched.source[0], local()[0]);
    }

    #[tokio::test]
    async fn without_llm_the_local_marks_are_kept() {
        let store = Store::new(Ok(Settings::default()), std::path::PathBuf::new());
        let finisher = Finisher::new(store, &Settings::default(), None);
        let phase = Settings::default().plan().finals;
        let heard = Heard {
            id: "1".into(),
            text: "拡張マガジンを取る".into(),
            context: Vec::new(),
            tokens: local(),
        };
        let enriched = finisher.finish(heard, &phase, None, |_| {}).await.unwrap();
        assert!(enriched.translations.is_empty());
        assert_eq!(enriched.source[0], local()[0]);
    }
}
