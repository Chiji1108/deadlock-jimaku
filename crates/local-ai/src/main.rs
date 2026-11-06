//! 途中の字幕と途中の翻訳を PC の中で作る（DESIGN.md 9.6）。アプリが起動し、標準入出力でやりとりする。
//!
//! - 引数: モデルを置くフォルダ、翻訳先（`en,ru` のようにコンマで区切る）、
//!   確定の字幕も作るなら `final`、作らないなら `partial`
//! - 入力: 1 バイトの種類（区切りはこれだけ）と、4 バイトの長さ（リトルエンディアン）と中身。
//!   中身は 16kHz の f32 の音声、読みと用語を付ける `{発話の id}\n{文}`、
//!   用語の表（`terms::Term` の JSON の配列。辞書が変わるたびに届き、入れ替える）のどれか
//! - 出力: 1 行に 1 つの JSON（`progress`、`ready`、`text`、`final`、`marked`）。`final` は区切りごとに
//!   1 つ返す。文字はアプリの `Token` の形で入る
//!
//! 入力が閉じたら終わる（アプリが落ちても残らない）

mod furigana;
mod models;
mod terms;

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use furigana::{Furigana, Morpheme, Part, annotate};
use fxtranslate::engine::Engine;
use models::PairPaths;
use serde::Serialize;
use serde_json::{Value, json};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};
use terms::{Found, Terms};

const AUDIO: u8 = 1;
const CUT: u8 = 2;
const MARK: u8 = 3;
const TERMS: u8 = 4;
const SAMPLE_RATE: usize = 16_000;
/// 長い発話でも、再デコードの時間が伸び続けないようにする
const MAX_SAMPLES: usize = 10 * SAMPLE_RATE;
/// 短い発話は滑らかに更新する。長くなったら認識の時間の 1.25 倍に広げ、CPU を占有しない
const MIN_TICK: Duration = Duration::from_millis(200);

/// 開いている発話の音声。最新だけあればいいので、デコードが追いつかなければ途中を飛ばす
#[derive(Default)]
struct Speech {
    samples: Vec<f32>,
    /// 区切るたびに増える。デコード中に区切られたら結果を捨てる
    generation: u64,
    /// 前のデコードのあとに音声が増えた
    fresh: bool,
    /// 区切られた発話。確定の字幕を作る
    finished: VecDeque<Vec<f32>>,
    /// OpenAI の確定の文（発話の id と文）。読みと用語を付けて返す
    marks: VecDeque<(String, String)>,
    /// 新しい用語の表
    terms: Option<Terms>,
    closed: bool,
}

/// 次にすること。確定の字幕を先に作る
enum Job {
    Terms(Terms),
    Final(Vec<f32>),
    Mark(String, String),
    Partial(Vec<f32>, u64),
}

impl Speech {
    fn next_job(&mut self) -> Option<Job> {
        if let Some(terms) = self.terms.take() {
            return Some(Job::Terms(terms));
        }
        if let Some(finished) = self.finished.pop_front() {
            return Some(Job::Final(finished));
        }
        if let Some((id, text)) = self.marks.pop_front() {
            return Some(Job::Mark(id, text));
        }
        let start = self.samples.len().saturating_sub(MAX_SAMPLES);
        std::mem::take(&mut self.fresh)
            .then(|| Job::Partial(self.samples[start..].to_vec(), self.generation))
    }
}

/// アプリの `Token` と同じ形
#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum Token {
    Text(Part),
    Entity {
        #[serde(rename = "ref")]
        reference: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        spoken: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ruby: Option<Vec<Part>>,
    },
}

type Shared = Arc<(Mutex<Speech>, Condvar)>;

/// 字幕を作るもの。Engine はスレッドをまたげないので、メインのスレッドで持つ
struct Local {
    recognizer: OfflineRecognizer,
    furigana: Furigana,
    japanese_english: Engine,
    /// 英語から訳す言語
    from_english: Vec<(String, Engine)>,
    translate_english: bool,
    terms: Terms,
}

impl Local {
    fn decode(&self, samples: &[f32]) -> String {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE as i32, samples);
        self.recognizer.decode(&stream);
        stream
            .get_result()
            .map(|r| r.text.trim().to_owned())
            .unwrap_or_default()
    }

    /// 用語を先に英語にしておくと、訳が崩れない（「ヘイズ」が Hayes にならない）
    fn translate(&self, text: &str, found: &[Found]) -> BTreeMap<String, Vec<Token>> {
        let english = self
            .japanese_english
            .translate(&with_english_names(text, found));
        let translated = self
            .from_english
            .iter()
            .map(|(lang, engine)| (lang.as_str(), engine.translate(&english)));
        let english = self.translate_english.then_some(("en", english.clone()));
        translation_tokens(translated.chain(english), found)
    }

    fn message(&self, kind: &str, text: &str) -> Value {
        let morphemes = self.furigana.morphemes(text);
        let found = self.terms.find(text, &morphemes);
        json!({
            "type": kind,
            "text": text,
            "tokens": source_tokens(text, &morphemes, &found),
            "translations": self.translate(text, &found),
        })
    }

    /// 訳は OpenAI のものを待つので、原文に読みと用語だけを付ける
    fn mark(&self, id: &str, text: &str) -> Value {
        let morphemes = self.furigana.morphemes(text);
        let found = self.terms.find(text, &morphemes);
        json!({
            "type": "marked",
            "id": id,
            "tokens": source_tokens(text, &morphemes, &found),
        })
    }
}

/// 話した言葉はそのまま残し、用語にアイコンと色が付くようにする
fn source_tokens(text: &str, morphemes: &[Morpheme], found: &[Found]) -> Vec<Token> {
    let plain = |from, to| {
        annotate(text, morphemes, from, to)
            .into_iter()
            .map(Token::Text)
    };
    let mut tokens = Vec::new();
    let mut pos = 0;
    for f in found {
        tokens.extend(plain(pos, f.start));
        let spoken = &text[f.start..f.end];
        tokens.push(Token::Entity {
            reference: f.term.reference.clone(),
            spoken: Some(spoken.to_owned()),
            ruby: terms::ruby(f.term, spoken),
        });
        pos = f.end;
    }
    tokens.extend(plain(pos, text.len()));
    tokens
}

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

/// 言語ごとの訳文をトークンにする。訳が空の言語は載せない（訳文の行が空のまま出る）
fn translation_tokens<'a>(
    translated: impl Iterator<Item = (&'a str, String)>,
    found: &[Found],
) -> BTreeMap<String, Vec<Token>> {
    translated
        .map(|(lang, text)| (lang.to_owned(), mark_names(&text, found)))
        .filter(|(_, tokens)| !tokens.is_empty())
        .collect()
}

/// 訳文に残った英語名を用語にする。原文で見つけた用語だけを探す
fn mark_names(translation: &str, found: &[Found]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut rest = translation;
    while let Some((start, term)) = found
        .iter()
        .filter_map(|f| Some((find_word(rest, &f.term.en)?, f.term)))
        .min_by_key(|(start, term)| (*start, std::cmp::Reverse(term.en.len())))
    {
        push_text(&mut tokens, &rest[..start]);
        tokens.push(Token::Entity {
            reference: term.reference.clone(),
            spoken: None,
            ruby: None,
        });
        rest = &rest[start + term.en.len()..];
    }
    push_text(&mut tokens, rest);
    tokens
}

/// 別の単語の一部（Rem と Remember）は名前にしない
fn find_word(text: &str, word: &str) -> Option<usize> {
    let joined = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    text.match_indices(word)
        .find(|(i, _)| {
            !joined(text[..*i].chars().next_back())
                && !joined(text[i + word.len()..].chars().next())
        })
        .map(|(i, _)| i)
}

fn push_text(tokens: &mut Vec<Token>, text: &str) {
    if !text.is_empty() {
        tokens.push(Token::Text(Part {
            text: text.to_owned(),
            ruby: None,
        }));
    }
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().context("models folder")?);
    let targets: Vec<String> = args
        .next()
        .context("targets")?
        .split(',')
        .filter(|t| !t.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    let finals = args.next().context("mode")? == "final";

    let others: Vec<String> = targets.iter().filter(|t| *t != "en").cloned().collect();
    let paths = models::ensure(&dir, &others, |loaded, total| {
        emit(&json!({ "type": "progress", "loaded": loaded, "total": total }));
    })?;
    let mut pairs = paths.pairs.iter();
    let (_, japanese_english) = pairs.next().context("ja-en model")?;
    let mut local = Local {
        recognizer: recognizer(&paths)?,
        furigana: Furigana::new()?,
        japanese_english: engine(japanese_english)?,
        from_english: pairs
            .map(|(name, pair)| {
                let lang = name.trim_start_matches("en-").to_owned();
                Ok((lang, engine(pair)?))
            })
            .collect::<anyhow::Result<_>>()?,
        translate_english: targets.iter().any(|t| t == "en"),
        terms: Terms::new(Vec::new()),
    };
    emit(&json!({ "type": "ready" }));

    let speech: Shared = Arc::default();
    let reader = speech.clone();
    std::thread::spawn(move || {
        if let Err(e) = read_input(&reader, finals) {
            eprintln!("input: {e}");
        }
        reader.0.lock().unwrap().closed = true;
        reader.1.notify_all();
    });

    let mut shown = String::new();
    let mut shown_generation = 0;
    loop {
        let job = {
            let (lock, wake) = &*speech;
            wake.wait_while(lock.lock().unwrap(), |s| {
                !s.fresh
                    && !s.closed
                    && s.finished.is_empty()
                    && s.marks.is_empty()
                    && s.terms.is_none()
            })
            .unwrap()
            .next_job()
        };
        let (samples, generation) = match job {
            None => return Ok(()),
            Some(Job::Terms(terms)) => {
                local.terms = terms;
                continue;
            }
            Some(Job::Final(finished)) => {
                let text = local.decode(&finished);
                emit(&local.message("final", &text));
                continue;
            }
            Some(Job::Mark(id, text)) => {
                emit(&local.mark(&id, &text));
                continue;
            }
            Some(Job::Partial(samples, generation)) => (samples, generation),
        };
        let started = Instant::now();
        let text = local.decode(&samples);
        if generation != shown_generation {
            shown.clear();
            shown_generation = generation;
        }
        if !text.is_empty() && text != shown && speech.0.lock().unwrap().generation == generation {
            emit(&local.message("text", &text));
            shown = text;
        }
        let elapsed = started.elapsed();
        std::thread::sleep(MIN_TICK.max(elapsed.mul_f32(1.25)).saturating_sub(elapsed));
    }
}

fn recognizer(paths: &models::Paths) -> anyhow::Result<OfflineRecognizer> {
    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice.model = Some(path(&paths.speech_model));
    config.model_config.sense_voice.language = Some("ja".to_owned());
    config.model_config.sense_voice.use_itn = false;
    config.model_config.tokens = Some(path(&paths.tokens));
    // ゲームと同時に動くので、1 スレッドに留める
    config.model_config.num_threads = 1;
    OfflineRecognizer::create(&config).context("load the speech model")
}

fn engine(pair: &PairPaths) -> anyhow::Result<Engine> {
    let shortlist = std::fs::read(&pair.shortlist)?;
    let engine = Engine::load_mmapped(
        &pair.model,
        &pair.source_vocabulary,
        &pair.target_vocabulary,
    )
    .map_err(|e| anyhow::anyhow!("load the translation model: {e:?}"))?;
    Ok(engine.with_shortlist_bytes(&shortlist))
}

fn path(path: &Path) -> String {
    path.display().to_string()
}

fn read_input(speech: &Shared, finals: bool) -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut kind = [0];
    while input.read_exact(&mut kind).is_ok() {
        match kind[0] {
            AUDIO => {
                let bytes = read_body(&mut input)?;
                let mut s = speech.0.lock().unwrap();
                s.samples.extend(
                    bytes
                        .chunks_exact(4)
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                );
                s.fresh = true;
                speech.1.notify_all();
            }
            CUT => {
                let mut s = speech.0.lock().unwrap();
                let samples = std::mem::take(&mut s.samples);
                if finals {
                    s.finished.push_back(samples);
                }
                s.generation += 1;
                s.fresh = false;
                speech.1.notify_all();
            }
            MARK => {
                let frame = String::from_utf8(read_body(&mut input)?).map_err(io::Error::other)?;
                if let Some((id, text)) = frame.split_once('\n') {
                    let mut s = speech.0.lock().unwrap();
                    s.marks.push_back((id.to_owned(), text.to_owned()));
                    speech.1.notify_all();
                }
            }
            TERMS => {
                let terms = serde_json::from_slice(&read_body(&mut input)?)?;
                speech.0.lock().unwrap().terms = Some(Terms::new(terms));
                speech.1.notify_all();
            }
            other => return Err(io::Error::other(format!("unknown input {other}"))),
        }
    }
    Ok(())
}

fn read_body(input: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0; 4];
    input.read_exact(&mut len)?;
    let mut bytes = vec![0; u32::from_le_bytes(len) as usize];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn emit(message: &Value) {
    let mut out = BufWriter::new(io::stdout().lock());
    let _ = writeln!(out, "{message}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use terms::Term;

    fn magazine() -> Term {
        Term {
            reference: "item:1".into(),
            text: "基礎拡張マガジン".into(),
            reading: "きそかくちょうマガジン".into(),
            en: "Extended Magazine".into(),
        }
    }

    fn text(text: &str) -> Token {
        Token::Text(Part {
            text: text.into(),
            ruby: None,
        })
    }

    fn entity(spoken: Option<&str>) -> Token {
        Token::Entity {
            reference: "item:1".into(),
            spoken: spoken.map(Into::into),
            ruby: None,
        }
    }

    #[test]
    fn new_terms_come_before_the_next_subtitle() {
        let mut speech = Speech {
            finished: VecDeque::from([Vec::new()]),
            terms: Some(Terms::new(vec![magazine()])),
            ..Default::default()
        };
        assert!(matches!(speech.next_job(), Some(Job::Terms(_))));
        assert!(matches!(speech.next_job(), Some(Job::Final(_))));
    }

    #[test]
    fn the_translator_receives_the_english_name() {
        let term = magazine();
        let found = [Found {
            start: 0,
            end: "きそかくちょうまがじん".len(),
            term: &term,
        }];
        assert_eq!(
            with_english_names("きそかくちょうまがじんを取る", &found),
            "Extended Magazineを取る"
        );
    }

    #[test]
    fn translations_show_the_english_name_as_a_term() {
        let term = magazine();
        let found = [Found {
            start: 0,
            end: 1,
            term: &term,
        }];
        assert_eq!(
            mark_names("Buy Extended Magazine first", &found),
            [text("Buy "), entity(None), text(" first")]
        );
        assert_eq!(mark_names("Buy a clip", &[]), [text("Buy a clip")]);
    }

    #[test]
    fn a_name_inside_another_word_stays_plain() {
        let mut term = magazine();
        term.en = "Rem".into();
        let found = [Found {
            start: 0,
            end: 1,
            term: &term,
        }];
        assert_eq!(mark_names("Remember", &found), [text("Remember")]);
        assert_eq!(
            mark_names("Rem's Remember", &found),
            [entity(None), text("'s Remember")]
        );
    }

    #[test]
    fn the_source_keeps_what_was_said() {
        let furigana = Furigana::new().unwrap();
        let term = magazine();
        let sentence = "きそかくちょうまがじんを";
        let found = [Found {
            start: 0,
            end: "きそかくちょうまがじん".len(),
            term: &term,
        }];
        let tokens = source_tokens(sentence, &furigana.morphemes(sentence), &found);
        assert_eq!(tokens, [entity(Some("きそかくちょうまがじん")), text("を")]);
    }

    #[test]
    fn a_language_without_a_translation_is_left_out() {
        let translated = [("en", "Reload".to_owned()), ("ru", String::new())];
        let tokens = translation_tokens(translated.into_iter(), &[]);
        assert_eq!(tokens.keys().collect::<Vec<_>>(), ["en"]);
        assert_eq!(tokens["en"], [text("Reload")]);
    }

    #[test]
    fn tokens_have_the_shape_of_the_app() {
        assert_eq!(
            serde_json::to_value(text("あ")).unwrap(),
            json!({ "t": "text", "text": "あ" })
        );
        assert_eq!(
            serde_json::to_value(entity(Some("きそかくちょうまがじん"))).unwrap(),
            json!({ "t": "entity", "ref": "item:1", "spoken": "きそかくちょうまがじん" })
        );
    }
}
