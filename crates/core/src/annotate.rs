//! 認識した文に、PC の中でふりがなと用語を付ける（DESIGN.md 9.6）。
//!
//! 1 文 1ms ほどなので、文を受け取った場所で同期で呼び、文とトークンを同時に発話へ載せる。
//! 何を付けるかは `Plan` の `Phase` で決まる。
//! 翻訳は Chrome のページがするので、用語を英語の名前に置き換えた文（`Translatable`）も作る。
//! 形態素解析の辞書は日本語だけなので、ほかの言語の文はそのまま返す

use std::sync::Arc;

use tokio::sync::watch;

use crate::assets::Loaded;
use crate::people::{self, FirstPerson};
use crate::reading::{Furigana, Morpheme, annotate};
use crate::settings::{Phase, Settings};
use crate::store::Store;
use crate::terms::{Found, Term, Terms, ruby};
use crate::types::{
    Entity, Glossary, LiveState, LiveStatus, RubySegment as Part, TermName, Token, Translatable,
};

/// 字幕の処理が持つ。あるあいだ、ページの文を受け取れる（`live.status`）
pub struct Annotator {
    store: Arc<Store>,
    furigana: Option<&'static Furigana>,
    /// デッドロックモードのときだけ。辞書はあとから届いたり、読みが増えて作り直されたりする
    glossary: Option<watch::Receiver<Option<Loaded>>>,
    terms: Terms,
}

impl Annotator {
    pub(crate) async fn new(store: Arc<Store>, settings: &Settings) -> Self {
        // 辞書を広げるのは最初の 1 回だけ時間がかかる
        let furigana = if settings.mic_language == "ja" {
            tokio::task::spawn_blocking(Furigana::shared)
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        let mut glossary = settings.deadlock_mode.then(|| store.watch_glossary());
        if let Some(glossary) = &mut glossary {
            glossary.mark_changed();
        }
        store.set_live_status(Some(LiveStatus {
            state: LiveState::Listening,
            detail: None,
        }));
        Self {
            store,
            furigana,
            glossary,
            terms: Terms::new(Vec::new()),
        }
    }

    /// `phase` のとおりに読みと用語を付けた文と、翻訳に渡す文
    pub fn annotate(&mut self, text: &str, phase: &Phase) -> (Vec<Token>, Translatable) {
        if let Some(glossary) = self
            .glossary
            .as_mut()
            .filter(|g| g.has_changed().unwrap_or(false))
        {
            let table = glossary.borrow_and_update();
            let table = table.as_ref().map(|l| table_of(&l.glossary, self.furigana));
            self.terms = Terms::new(table.unwrap_or_default());
        }
        let with_terms = phase.local_terms();
        // ふりがなを LLM や判定モデルに直させるときも、まず辞書の読みを付ける（直らなかった語と、聞けなかった文に残る）
        let with_ruby = phase.ruby.is_some();
        let furigana = self.furigana.filter(|_| with_ruby || with_terms);
        let terms = with_terms.then_some(&self.terms);
        let (tokens, translate) = annotated(furigana, terms, with_ruby, text);
        // 用語を強調するなら、自分を指す言葉（一人称）にもアイコンを付ける。翻訳に渡す文は変えない
        let me = people::first_person(&self.store.settings().people);
        let tokens = match (self.furigana.filter(|_| with_terms), me) {
            (Some(furigana), Some(me)) => mark_first_person(furigana, text, tokens, &me),
            _ => tokens,
        };
        (tokens, translate)
    }
}

impl Drop for Annotator {
    fn drop(&mut self) {
        self.store.set_live_status(None);
    }
}

fn annotated(
    furigana: Option<&Furigana>,
    terms: Option<&Terms>,
    with_ruby: bool,
    text: &str,
) -> (Vec<Token>, Translatable) {
    let morphemes = furigana.map(|f| f.morphemes(text)).unwrap_or_default();
    let found = terms.map(|t| t.find(text, &morphemes)).unwrap_or_default();
    let translate = Translatable {
        text: with_english_names(text, &found),
        terms: found
            .iter()
            .map(|f| TermName {
                reference: f.term.reference.clone(),
                en: f.term.en.clone(),
            })
            .collect(),
    };
    let spoken = source_tokens(text, &morphemes, &found, with_ruby);
    (spoken, translate)
}

/// 自分を指す言葉（一人称）を、自分の用語にする（DESIGN.md 9.12）。原文の見た目だけを変える: 用語を決めて、
/// 翻訳に渡す文を作ったあとに呼ぶので、翻訳では名前に置き換わらない（"I" のまま）。
/// 言葉の区切りで見るので、「私立」「下僕」「そのうち」には当たらない。意味は見ない（「私」「俺」は、ほぼ必ず自分を指す）
pub fn mark_first_person(
    furigana: &Furigana,
    text: &str,
    tokens: Vec<Token>,
    me: &FirstPerson,
) -> Vec<Token> {
    let morphemes = furigana.morphemes(text);
    let is_word = |m: &&Morpheme| me.words.iter().any(|word| *word == text[m.start..m.end]);
    let words: Vec<&Morpheme> = morphemes.iter().filter(is_word).collect();
    if words.is_empty() {
        return tokens;
    }
    let mut marked = Vec::new();
    let mut pos = 0;
    for token in tokens {
        let Token::Text { text: part, ruby } = token else {
            // もう用語になっている所は、そのまま
            let Token::Entity { spoken, .. } = &token else {
                unreachable!()
            };
            pos += spoken.as_ref().map_or(0, String::len);
            marked.push(token);
            continue;
        };
        let (start, end) = (pos, pos + part.len());
        pos = end;
        // 文字の並びが原文とずれていたら、何もしない
        if text.get(start..end) != Some(part.as_str()) {
            marked.push(Token::Text { text: part, ruby });
            continue;
        }
        let inside: Vec<&&Morpheme> = (words.iter())
            .filter(|m| start <= m.start && m.end <= end)
            .collect();
        // ふりがなの付いた所は、まるごと一人称のときだけ（「私」に「わたし」）
        let whole = inside.len() == 1 && inside[0].start == start && inside[0].end == end;
        if inside.is_empty() || (ruby.is_some() && !whole) {
            marked.push(Token::Text { text: part, ruby });
            continue;
        }
        let plain = |from: usize, to: usize| Token::Text {
            text: text[from..to].to_owned(),
            ruby: None,
        };
        let mut at = start;
        for word in inside {
            if at < word.start {
                marked.push(plain(at, word.start));
            }
            let spoken = text[word.start..word.end].to_owned();
            marked.push(Token::Entity {
                reference: me.reference.clone(),
                ruby: ruby.clone().map(|ruby| {
                    vec![Part {
                        text: spoken.clone(),
                        ruby: Some(ruby),
                    }]
                }),
                spoken: Some(spoken),
            });
            at = word.end;
        }
        if at < end {
            marked.push(plain(at, end));
        }
    }
    marked
}

/// 話した言葉はそのまま残し、用語にアイコンと色が付くようにする
fn source_tokens(
    text: &str,
    morphemes: &[Morpheme],
    found: &[Found],
    with_ruby: bool,
) -> Vec<Token> {
    let plain = |from, to| {
        annotate(text, morphemes, from, to)
            .into_iter()
            .map(|part| Token::Text {
                text: part.text,
                ruby: part.ruby.filter(|_| with_ruby),
            })
    };
    let mut tokens = Vec::new();
    let mut pos = 0;
    for f in found {
        tokens.extend(plain(pos, f.start));
        let spoken = &text[f.start..f.end];
        tokens.push(Token::Entity {
            reference: f.term.reference.clone(),
            spoken: Some(spoken.to_owned()),
            ruby: ruby(f.term, spoken).filter(|_| with_ruby),
        });
        pos = f.end;
    }
    tokens.extend(plain(pos, text.len()));
    tokens
}

/// 用語を先に英語にしておくと、訳が崩れない（「ヘイズ」が Hayes にならない）
fn with_english_names(text: &str, found: &[Found]) -> String {
    let mut replaced = String::with_capacity(text.len());
    let mut pos = 0;
    for f in found {
        replaced.push_str(&text[pos..f.start]);
        replaced.push_str(&f.term.en);
        pos = f.end;
    }
    replaced.push_str(&text[pos..]);
    replaced
}

/// 日本語と英語の公式名がある、すべての用語。読みは LLM のもの、なければ辞書が名前を読んだもの。
/// 辞書が読み誤った名前は読みでは見つからないだけで、書き方の一致は効く
fn table_of(glossary: &Glossary, furigana: Option<&Furigana>) -> Vec<Term> {
    let mut terms: Vec<Term> = glossary
        .entities
        .iter()
        .filter_map(|(reference, e)| {
            let (name, en) = (e.names.get("ja")?, e.names.get("en")?);
            Some(Term {
                reference: reference.clone(),
                text: name.clone(),
                reading: reading(e)
                    .or_else(|| furigana?.reading(name))
                    .unwrap_or_else(|| name.clone()),
                en: en.clone(),
            })
        })
        .collect();
    // 人の呼ばれ方（あだ名）も、名前と同じように探す
    for (reference, e) in &glossary.entities {
        let Some(en) = e.names.get("en") else {
            continue;
        };
        terms.extend(e.aliases.iter().map(|alias| {
            Term {
                reference: reference.clone(),
                text: alias.clone(),
                reading: furigana
                    .and_then(|f| f.reading(alias))
                    .unwrap_or_else(|| alias.clone()),
                en: en.clone(),
            }
        }));
    }
    // スキルと同じ名前のアイテムがある（ライフドレイン）。先の行が勝つので、説明を出せるアイテムを先にする
    terms.sort_by_key(|t| !t.reference.starts_with("item:"));
    terms
}

fn reading(entity: &Entity) -> Option<String> {
    let ruby = entity.ruby.as_ref()?.iter();
    Some(
        ruby.map(|s| s.ruby.as_ref().unwrap_or(&s.text).as_str())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EntityKind, RubySegment};

    /// アプリの保存した辞書で、用語のテストの文（`experiments/results/terms_cases.json`）に PC の中の一致を当てて、
    /// 正解と比べる。読みの辞書を替えたときに、前と比べる:
    /// `cargo test -p overlays-core local_terms_on_the_cases -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "reads the app's glossary"]
    async fn local_terms_on_the_cases() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let languages = ["ja".to_owned(), "en".to_owned()];
        let loaded = crate::assets::load(&crate::default_data_dir().join("assets"), &languages)
            .await
            .unwrap();
        let furigana = Furigana::shared().unwrap();
        let terms = Terms::new(table_of(&loaded.glossary, Some(furigana)));
        let cases = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../experiments/results/terms_cases.json"
        );
        let cases: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(cases).unwrap()).unwrap();
        let mut right = 0;
        for case in &cases {
            let text = case["text"].as_str().unwrap();
            let segments = case["llm"][0]["answer"]["segments"].as_array().unwrap();
            let gold: Vec<(String, String)> = segments
                .iter()
                .filter_map(|s| {
                    Some((
                        s["text"].as_str()?.to_owned(),
                        s["ref"].as_str()?.to_owned(),
                    ))
                })
                .collect();
            let (tokens, _) = annotated(Some(furigana), Some(&terms), false, text);
            let found: Vec<(String, String)> = tokens
                .iter()
                .filter_map(|token| match token {
                    Token::Entity {
                        reference, spoken, ..
                    } => Some((spoken.clone().unwrap_or_default(), reference.clone())),
                    Token::Text { .. } => None,
                })
                .collect();
            if found == gold {
                right += 1;
            } else {
                println!("{text}\n  found {found:?}\n  gold  {gold:?}");
            }
        }
        println!("local terms: {right}/{} sentences", cases.len());
    }

    fn magazine(ruby: Option<Vec<RubySegment>>) -> Glossary {
        let entity = Entity {
            kind: EntityKind::Item,
            class_name: "upgrade_clip_size".into(),
            names: [("ja", "基礎拡張マガジン"), ("en", "Extended Magazine")]
                .map(|(lang, name)| (lang.to_owned(), name.to_owned()))
                .into(),
            ruby,
            icon: None,
            slot: None,
            hero: None,
            ability_slot: None,
            aliases: Vec::new(),
        };
        Glossary {
            client_version: 1,
            languages: Vec::new(),
            entities: [("item:1548066885".to_owned(), entity)].into(),
        }
    }

    fn term() -> Term {
        Term {
            reference: "item:1".into(),
            text: "基礎拡張マガジン".into(),
            reading: "きそかくちょうマガジン".into(),
            en: "Extended Magazine".into(),
        }
    }

    #[test]
    fn a_term_has_the_reading_of_its_name() {
        let ruby = vec![
            RubySegment {
                text: "基礎拡張".into(),
                ruby: Some("きそかくちょう".into()),
            },
            RubySegment {
                text: "マガジン".into(),
                ruby: None,
            },
        ];
        let terms = table_of(&magazine(Some(ruby)), None);
        assert_eq!(terms[0].reading, "きそかくちょうマガジン");
        assert_eq!(terms[0].en, "Extended Magazine");
    }

    /// 一人称は、言葉の区切りで見て、自分の用語にする。ほかの言葉の一部や、用語の中には当てない
    #[test]
    fn first_person_words_become_the_streamer() {
        let furigana = Furigana::shared().unwrap();
        let me = FirstPerson {
            reference: "person:me".into(),
            words: vec!["私".into(), "俺".into(), "僕".into()],
        };
        let marked = |text: &str, with_ruby: bool| {
            let (tokens, translate) = annotated(Some(furigana), None, with_ruby, text);
            assert_eq!(translate.text, text);
            mark_first_person(furigana, text, tokens, &me)
        };
        let words = |tokens: &[Token]| -> Vec<String> {
            let entity = |token: &Token| match token {
                Token::Entity { spoken, .. } => spoken.clone(),
                Token::Text { .. } => None,
            };
            tokens.iter().filter_map(entity).collect()
        };
        let whole = |tokens: &[Token]| -> String {
            let text = |token: &Token| match token {
                Token::Entity { spoken, .. } => spoken.clone().unwrap_or_default(),
                Token::Text { text, .. } => text.clone(),
            };
            tokens.iter().map(text).collect()
        };
        for (text, found) in [
            ("俺が行くよ", vec!["俺"]),
            ("私は下がるから僕に任せて", vec!["私", "僕"]),
            // ほかの言葉の一部
            ("私立の学校と下僕の話", vec![]),
            ("ウルト使うよ", vec![]),
        ] {
            for with_ruby in [false, true] {
                let tokens = marked(text, with_ruby);
                assert_eq!(words(&tokens), found, "{text}: {tokens:?}");
                // 文字は変えない
                assert_eq!(whole(&tokens), text);
            }
        }
        // ふりがなは、一人称に付けたまま
        let tokens = marked("私は行く", true);
        let Token::Entity {
            reference, ruby, ..
        } = &tokens[0]
        else {
            panic!("{tokens:?}");
        };
        assert_eq!(reference, "person:me");
        assert_eq!(ruby.as_ref().unwrap()[0].ruby.as_deref(), Some("わたし"));
    }

    /// 人の辞書の人は、名前でも、呼ばれ方でも見つかる。翻訳に渡す文では、その人の英語の名前になる
    #[test]
    fn a_person_is_found_by_name_and_by_what_they_are_called() {
        let mut glossary = magazine(None);
        glossary.languages = vec!["ja".into(), "en".into()];
        let me = crate::people::Person {
            id: "me".into(),
            me: true,
            names: [("ja", "ちじ"), ("en", "Chiji")]
                .map(|(lang, name)| (lang.to_owned(), name.to_owned()))
                .into(),
            aliases: vec!["ちーさん".into()],
            icon: None,
            first_person: Vec::new(),
        };
        crate::people::apply(&mut glossary, &[me]);
        let furigana = Furigana::shared().unwrap();
        let terms = Terms::new(table_of(&glossary, Some(furigana)));
        for said in ["ちじが行くよ", "ちーさんが行くよ"] {
            let (tokens, translate) = annotated(Some(furigana), Some(&terms), false, said);
            let found = tokens.iter().any(|token| {
                matches!(token, Token::Entity { reference, .. } if reference == "person:me")
            });
            assert!(found, "{said}: {tokens:?}");
            assert!(translate.text.starts_with("Chiji"), "{}", translate.text);
        }
    }

    #[test]
    fn a_name_that_is_an_ordinary_word_is_a_term() {
        let mut glossary = magazine(None);
        let entity = glossary.entities.values_mut().next().unwrap();
        entity.names.insert("ja".into(), "ポケット".into());
        assert_eq!(table_of(&glossary, None)[0].text, "ポケット");
    }

    #[test]
    fn items_come_before_abilities_of_the_same_name() {
        let mut glossary = magazine(None);
        let mut ability = glossary.entities.values().next().unwrap().clone();
        ability.kind = EntityKind::Ability;
        glossary.entities.insert("ability:1".into(), ability);
        assert_eq!(table_of(&glossary, None)[0].reference, "item:1548066885");
    }

    #[test]
    fn a_name_without_a_reading_is_listed_as_written() {
        assert_eq!(
            table_of(&magazine(None), None)[0].reading,
            "基礎拡張マガジン"
        );
    }

    #[test]
    fn without_a_cached_reading_the_dictionary_reads_the_name() {
        let furigana = Furigana::shared().unwrap();
        let mut glossary = magazine(None);
        let entity = glossary.entities.values_mut().next().unwrap();
        entity.names.insert("ja".into(), "呪いの遺物".into());
        assert_eq!(
            table_of(&glossary, Some(furigana))[0].reading,
            "のろいのいぶつ"
        );
    }

    #[test]
    fn the_translator_receives_the_english_name_and_the_source_keeps_what_was_said() {
        let terms = Terms::new(vec![term()]);
        let furigana = Furigana::shared().unwrap();
        let (tokens, translate) =
            annotated(Some(furigana), Some(&terms), true, "基礎拡張マガジンを取る");
        assert_eq!(translate.text, "Extended Magazineを取る");
        assert_eq!(translate.terms[0].reference, "item:1");
        assert!(matches!(
            &tokens[0],
            Token::Entity { reference, spoken: Some(spoken), .. }
                if reference == "item:1" && spoken == "基礎拡張マガジン"
        ));
    }

    #[test]
    fn without_the_dictionary_the_text_stays_as_it_is() {
        let terms = Terms::new(vec![term()]);
        let (tokens, translate) = annotated(None, Some(&terms), true, "Reload now");
        assert_eq!(
            tokens,
            [Token::Text {
                text: "Reload now".into(),
                ruby: None,
            }]
        );
        assert_eq!(translate.text, "Reload now");
        assert!(translate.terms.is_empty());
    }

    #[test]
    fn without_ruby_the_terms_still_work() {
        let furigana = Furigana::shared().unwrap();
        let terms = Terms::new(vec![term()]);
        let (tokens, _) = annotated(
            Some(furigana),
            Some(&terms),
            false,
            "基礎拡張マガジンを取る",
        );
        assert!(tokens.iter().all(|t| match t {
            Token::Text { ruby, .. } => ruby.is_none(),
            Token::Entity { ruby, .. } => ruby.is_none(),
        }));
        assert!(matches!(tokens[0], Token::Entity { .. }));
    }

    #[tokio::test]
    async fn the_table_follows_the_glossary_and_the_status_ends_with_the_annotator() {
        let store = Store::new(Ok(Settings::default()), std::path::PathBuf::new());
        let mut annotator = Annotator::new(store.clone(), &Settings::default()).await;
        assert!(
            store
                .current_state()
                .iter()
                .any(|m| matches!(m, crate::types::ServerMsg::LiveStatus { status: Some(_) }))
        );
        let phase = Settings::default().plan().partial;
        let (_, translate) = annotator.annotate("基礎拡張マガジンを取る", &phase);
        assert!(translate.terms.is_empty(), "辞書がまだ届いていない");
        store.set_glossary(Loaded {
            glossary: Arc::new(magazine(None)),
            requested: Vec::new(),
            root: std::path::PathBuf::new(),
            aliases: Default::default(),
        });
        let (_, translate) = annotator.annotate("基礎拡張マガジンを取る", &phase);
        assert_eq!(translate.terms.len(), 1);
        drop(annotator);
        assert!(
            store
                .current_state()
                .iter()
                .any(|m| matches!(m, crate::types::ServerMsg::LiveStatus { status: None }))
        );
    }

    /// 同期で呼ぶので、1 文が短いことを確かめる。出力は `cargo test --release annotation_speed -- --nocapture --ignored`
    #[test]
    #[ignore = "測定"]
    fn annotation_speed() {
        let started = std::time::Instant::now();
        let furigana = Furigana::shared().unwrap();
        println!("dictionary {:?}", started.elapsed());
        let terms = Terms::new(vec![term()]);
        let sentence =
            "基礎拡張マガジンを買ってからヘイズの後ろに回って、そのままウルトで倒しにいきます";
        let started = std::time::Instant::now();
        for _ in 0..1000 {
            std::hint::black_box(annotated(Some(furigana), Some(&terms), true, sentence));
        }
        println!("per sentence {:?}", started.elapsed() / 1000);
    }
}
