//! PC の中で付けるふりがな（DESIGN.md 6.4、9.6）。読みは形態素解析（vibrato と、話し言葉の UniDic）で決める。
//! 文脈で決まる読みは誤ることがあるが、速く、外に送らない。ふりがなを判定モデルや LLM にしていれば、
//! 確定の文で、文に合わない読みを直させる（`furigana.rs`）。
//!
//! 辞書は国立国語研究所の「現代話し言葉 UniDic」。助詞を省く話し言葉（「ヘイズ強い」）を、書き言葉の辞書より
//! 正しく読む。語のつながりやすさを小さく持つ形に組み立てて（`examples/build_dictionary.rs`）、アプリに入れる

use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

use flate2::read::GzDecoder;
use vibrato::{Dictionary, Tokenizer};

use crate::types::RubySegment as Part;

/// 組み立てた辞書（74 MB を圧縮したもの）。読みは素性の最後
const DICTIONARY: &[u8] = include_bytes!("../dictionary/unidic-csj.dic.gz");
/// 同じ文字で読みが 2 つ以上ある語（`文字\t読み,読み,…`。よく使う読みから）
const HOMOGRAPHS: &[u8] = include_bytes!("../dictionary/homographs.tsv.gz");

/// 1 か所の読みの候補の上限
const MAX_OPTIONS: usize = 6;

/// 字幕の 1 語。バイトの位置は元の文のもの
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Morpheme {
    pub start: usize,
    pub end: usize,
    /// ひらがなの読み。分からなければ None
    pub reading: Option<String>,
}

pub struct Furigana {
    tokenizer: Tokenizer,
    homographs: HashMap<String, Vec<String>>,
}

impl Furigana {
    /// アプリで 1 つだけ開く辞書。初めてのときに広げる（0.3 秒ほど）。アプリに入っている辞書なので、
    /// 読めなければ None（文をそのまま使う）
    pub fn shared() -> Option<&'static Self> {
        static FURIGANA: OnceLock<Option<Furigana>> = OnceLock::new();
        FURIGANA
            .get_or_init(|| {
                Self::new()
                    .inspect_err(|e| tracing::warn!("furigana: {e:#}"))
                    .ok()
            })
            .as_ref()
    }

    fn new() -> anyhow::Result<Self> {
        let dictionary = Dictionary::read(GzDecoder::new(DICTIONARY))?;
        let mut table = String::new();
        GzDecoder::new(HOMOGRAPHS).read_to_string(&mut table)?;
        let homographs = table.lines().filter_map(|line| {
            let (word, readings) = line.split_once('\t')?;
            let readings = readings.split(',').map(str::to_owned).collect();
            Some((word.to_owned(), readings))
        });
        Ok(Self {
            tokenizer: Tokenizer::new(dictionary),
            homographs: homographs.collect(),
        })
    }

    /// 文全体のひらがなの読み。読めない形態素があれば None
    pub fn reading(&self, text: &str) -> Option<String> {
        self.morphemes(text)
            .into_iter()
            .map(|m| m.reading)
            .collect()
    }

    /// 文を語に分けて、読みを付ける
    pub fn morphemes(&self, text: &str) -> Vec<Morpheme> {
        let mut worker = self.tokenizer.new_worker();
        worker.reset_sentence(text);
        worker.tokenize();
        let morphemes = (0..worker.num_tokens()).map(|i| {
            let token = worker.token(i);
            let range = token.range_byte();
            let kana = token.feature().rsplit(',').next().unwrap_or_default();
            Morpheme {
                reading: reading(&text[range.clone()], kana),
                start: range.start,
                end: range.end,
            }
        });
        morphemes.collect()
    }

    /// 辞書の選んだ分け方と、読みが分かれるところ（漢字を含む語だけ）。同じ文字の語の、辞書にある別の読みを
    /// 候補にする（「辛い」の「つらい」「からい」）
    pub fn alternatives(&self, text: &str) -> (Vec<Morpheme>, Vec<Alternatives>) {
        let best = self.morphemes(text);
        let alternatives = best.iter().enumerate().filter_map(|(i, m)| {
            let word = &text[m.start..m.end];
            // 数と、その直後の数え方は聞かない。読みが組で決まるので（一発、三日、四人）、1 語ずつ選ばせると崩れる
            let before = i.checked_sub(1).map(|i| &text[best[i].start..best[i].end]);
            if counts(word, false) || before.is_some_and(|w| counts(w, true)) {
                return None;
            }
            let own = m.reading.as_ref()?;
            // 語のかなに合わない読みは、ふりがなにできない
            let others = self.homographs.get(word)?.iter();
            let others = others.filter(|r| *r != own && align(word, r).is_some());
            let readings = std::iter::once(own).chain(others).take(MAX_OPTIONS);
            let options: Vec<Vec<Morpheme>> = readings
                .map(|reading| {
                    vec![Morpheme {
                        reading: Some(reading.clone()),
                        ..m.clone()
                    }]
                })
                .collect();
            (options.len() > 1).then_some(Alternatives {
                start: m.start,
                end: m.end,
                options,
            })
        });
        let alternatives = alternatives.collect();
        (best, alternatives)
    }
}

/// 数を表す語。`asking` なら、数をたずねる語（何、幾、数）も含める（「何人」の「人」は数え方）
fn counts(word: &str, asking: bool) -> bool {
    let numeral = |c: char| {
        c.is_ascii_digit() || "０１２３４５６７８９〇一二三四五六七八九十百千万億".contains(c)
    };
    let asks = |c: char| asking && "何幾数".contains(c);
    !word.is_empty() && word.chars().all(|c| numeral(c) || asks(c))
}

/// 読みが分かれるところ。`options` の先頭が、辞書の選んだ読み
#[derive(Debug)]
pub struct Alternatives {
    pub start: usize,
    pub end: usize,
    pub options: Vec<Vec<Morpheme>>,
}

/// 語のひらがなの読み。かなだけの語は、その文字のまま。辞書の読みがかなでなければ（記号など）None
fn reading(word: &str, dictionary: &str) -> Option<String> {
    if word.chars().all(is_kana) {
        return Some(to_hiragana(word));
    }
    let kana = !dictionary.is_empty() && dictionary.chars().all(is_kana);
    kana.then(|| to_hiragana(dictionary))
}

/// 語をつなげた読み。読めない語は、文字のまま
pub fn read(text: &str, morphemes: &[Morpheme]) -> String {
    morphemes
        .iter()
        .map(|m| m.reading.as_deref().unwrap_or(&text[m.start..m.end]))
        .collect()
}

/// 文の `from..to` にふりがなを付ける。形態素にならなかった文字（空白など）もそのまま残す
pub fn annotate(text: &str, morphemes: &[Morpheme], from: usize, to: usize) -> Vec<Part> {
    let mut out: Vec<Part> = Vec::new();
    let mut pos = from;
    for m in morphemes.iter().filter(|m| from <= m.start && m.end <= to) {
        if pos < m.start {
            push(&mut out, plain(&text[pos..m.start]));
        }
        for part in ruby_tokens(&text[m.start..m.end], m.reading.as_deref()) {
            push(&mut out, part);
        }
        pos = m.end;
    }
    if pos < to {
        push(&mut out, plain(&text[pos..to]));
    }
    out
}

/// ルビのない文字は前の部分につなげる
fn push(out: &mut Vec<Part>, part: Part) {
    if let (
        Some(Part {
            text: prev,
            ruby: None,
        }),
        None,
    ) = (out.last_mut(), &part.ruby)
    {
        prev.push_str(&part.text);
        return;
    }
    out.push(part);
}

fn plain(text: &str) -> Part {
    Part {
        text: text.into(),
        ruby: None,
    }
}

/// 語の読みを漢字の部分ごとに割り当てる。漢字のない語や、かな以外の読みにはルビを付けない。
/// 「強い / つよい」→「強(つよ)」「い」、「取り扱い / とりあつかい」→「取(と)」「り」「扱(あつか)」「い」
fn ruby_tokens(text: &str, ruby: Option<&str>) -> Vec<Part> {
    let Some(ruby) = ruby.filter(|r| !r.is_empty() && r.chars().all(is_kana)) else {
        return vec![plain(text)];
    };
    if !text.chars().any(is_kanji) {
        return vec![plain(text)];
    }
    align(text, &to_hiragana(ruby)).unwrap_or_else(|| vec![plain(text)])
}

/// 漢字の続く部分と、それ以外（かななど）の部分に分け、かなの部分を読みの中で探して区切る。
/// 読みとかなの部分が合わなければ None
pub fn align(text: &str, reading: &str) -> Option<Vec<Part>> {
    let mut runs: Vec<(bool, String)> = Vec::new();
    for c in text.chars() {
        match runs.last_mut() {
            Some((kanji, run)) if *kanji == is_kanji(c) => run.push(c),
            _ => runs.push((is_kanji(c), c.to_string())),
        }
    }
    let r: Vec<char> = reading.chars().collect();
    let mut pos = 0;
    let mut out = Vec::new();
    for (i, (kanji, run)) in runs.iter().enumerate() {
        if !kanji {
            let kana: Vec<char> = to_hiragana(run).chars().collect();
            if r.get(pos..pos + kana.len())? != kana.as_slice() {
                return None;
            }
            out.push(plain(run));
            pos += kana.len();
            continue;
        }
        // 漢字の読みは、次のかなの部分が読みに現れるところまで（漢字 1 文字に少なくとも 1 文字）
        let end = match runs.get(i + 1) {
            None => r.len(),
            Some((_, next)) => {
                let next: Vec<char> = to_hiragana(next).chars().collect();
                (pos + 1..=r.len().saturating_sub(next.len()))
                    .find(|&j| r[j..].starts_with(&next))?
            }
        };
        if end <= pos {
            return None;
        }
        out.push(Part {
            text: run.clone(),
            ruby: Some(r[pos..end].iter().collect()),
        });
        pos = end;
    }
    (pos == r.len()).then_some(out)
}

pub fn to_hiragana(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            // カタカナ（ァ〜ヶ）をひらがなに。長音符などはそのまま
            '\u{30A1}'..='\u{30F6}' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

pub fn is_kana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}')
}

pub fn is_kanji(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '々')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 読みが分かれるところの、文字と読みの候補
    fn alternatives(text: &str) -> Vec<(String, Vec<String>)> {
        let (_, found) = Furigana::shared().unwrap().alternatives(text);
        let listed = found.iter().map(|a| {
            let options = a.options.iter().map(|o| read(text, o)).collect();
            (text[a.start..a.end].to_owned(), options)
        });
        listed.collect()
    }

    #[test]
    fn alternatives_are_other_readings_of_the_same_words() {
        let of = |text: &str, word: &str| {
            let found = alternatives(text);
            let (_, options) = found.into_iter().find(|(w, _)| w == word).expect(word);
            options
        };
        // 先頭が、辞書の選んだ読み。ほかは、同じ文字の語の別の読み
        let spicy = of("このカレー辛い", "辛い");
        assert_eq!(spicy[0], "からい");
        assert!(spicy.contains(&"つらい".to_owned()));
        assert!(of("人気のない場所", "人気").contains(&"ひとけ".to_owned()));
        // 複合語の中でだけ出る濁った読みと、固有名詞の読みは候補にしない
        assert!(!spicy.contains(&"がらい".to_owned()));
        assert!(!of("金が足りない", "金").contains(&"きむ".to_owned()));
        // 数と、その直後の数え方は聞かない
        let asked = |text: &str| {
            alternatives(text)
                .into_iter()
                .map(|(w, _)| w)
                .collect::<Vec<_>>()
        };
        assert_eq!(asked("三日ぶりに一発で四人倒した"), Vec::<String>::new());
        assert_eq!(asked("三日ぶりに辛いカレー"), ["辛い"]);
        assert!(!asked("何人いるか見て").contains(&"人".to_owned()));
        // 読みが 1 つの語は聞かない
        assert!(
            alternatives("火力が上がってる")
                .iter()
                .all(|(w, _)| w != "火力")
        );
        // かなだけのところは聞かない
        assert!(alternatives("めっちゃつよい").is_empty());
    }

    fn p(text: &str, ruby: Option<&str>) -> Part {
        Part {
            text: text.into(),
            ruby: ruby.map(Into::into),
        }
    }

    #[test]
    fn ruby_goes_only_on_kanji() {
        assert_eq!(
            ruby_tokens("強い", Some("つよい")),
            [p("強", Some("つよ")), p("い", None)]
        );
        assert_eq!(
            ruby_tokens("取り扱い", Some("とりあつかい")),
            [
                p("取", Some("と")),
                p("り", None),
                p("扱", Some("あつか")),
                p("い", None)
            ]
        );
    }

    #[test]
    fn reads_a_sentence_with_the_dictionary() {
        let furigana = Furigana::shared().unwrap();
        let text = "実況です";
        let parts = annotate(text, &furigana.morphemes(text), 0, text.len());
        assert_eq!(parts, [p("実況", Some("じっきょう")), p("です", None)]);
    }

    #[test]
    fn spaces_stay_in_the_text() {
        let furigana = Furigana::shared().unwrap();
        let text = "Deadlock is fun";
        let parts = annotate(text, &furigana.morphemes(text), 0, text.len());
        assert_eq!(parts, [p(text, None)]);
    }
}
