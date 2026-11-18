//! 確定した文の、数字と句読点を整える（DESIGN.md 9.15）。日本語だけ。
//!
//! 形態素解析（`reading.rs`。ふりがなと同じ辞書）の語の区切りと品詞を使う。新しいモデルは使わない。
//! - 数字: 数の語のあとに数える言葉（秒、回、キル、ソウルなど）が続くときだけ、算用数字にする（「三十五秒」→「35秒」、
//!   「一万五千ソウル」→「1万5000ソウル」）。語の区切りで見るので、「一気」「一緒」「一瞬」は変えない。
//!   「一番」（いちばん好き）と「一人」（ひとりで）は、言い回しのことが多いので変えない
//! - 句読点: 句読点を書かない文字起こし（Chrome、ReazonSpeech）の文にだけ付ける。Chrome は話の間に空白を入れるので、
//!   空白を、言い切りの形（です、ます、ね、よ…）のあとなら「。」、ほかは「、」にする。助詞や助動詞の前の空白
//!   （「カース は」）と、名詞と名詞の間の空白（「デッドロック 実況」）と、「は」のあとの間は、ただ消す。文の初めの感動詞（「うわ」「よし」）のあとに「、」、終わりに「。」を付ける

use crate::reading::Furigana;

/// 数字と句読点を整えた文
pub fn tidy(furigana: &Furigana, text: &str) -> String {
    let text = numbers(furigana, text);
    punctuate(furigana, &text)
}

/// 数える言葉。数の語のすぐ後ろにあれば、算用数字にする
const COUNTERS: &[&str] = &[
    "秒",
    "分",
    "時間",
    "時",
    "回",
    "番",
    "人",
    "個",
    "体",
    "本",
    "枚",
    "匹",
    "円",
    "年",
    "月",
    "日",
    "位",
    "点",
    "倍",
    "割",
    "発",
    "杯",
    "件",
    "週",
    "歳",
    "才",
    "度",
    "キル",
    "デス",
    "ソウル",
    "パーセント",
    "%",
    "％",
    "連勝",
    "連敗",
    "連続",
    "対",
    "ダメージ",
    "レベル",
    "ヒット",
    "週間",
    "秒間",
    "分間",
    "か月",
];
/// 数える言葉が付いていても、言い回しのことが多い語（そのまま残す）
const IDIOMS: &[&str] = &[
    "一番",
    "一人",
    // じゅうぶん（10 分のことは少ない）
    "十分",
    "一度",
    "一本",
    "一発",
    "一回り",
    "一時",
    "一日",
    "一月",
];
/// 数の前に付けて、数と読ませる言葉（「レベル五」→「レベル5」）
const BEFORE: &[&str] = &["レベル", "ランク", "ラウンド", "第"];

fn is_numeral(c: char) -> bool {
    c.is_ascii_digit() || "０１２３４５６７８９〇零一二三四五六七八九十百千万億".contains(c)
}

/// 数の語と、その後ろの数える言葉を算用数字にする
fn numbers(furigana: &Furigana, text: &str) -> String {
    let words = furigana.words(text);
    let mut out = String::new();
    let mut pos = 0;
    let mut i = 0;
    while i < words.len() {
        // 続いている数の語（「三十」「五」）をまとめる
        let start = words[i].0.start;
        let mut j = i;
        while j < words.len() && text[words[j].0.clone()].chars().all(is_numeral) {
            j += 1;
        }
        if j == i {
            // 1 語の中に数と数える言葉がある（「二人」「三回」）
            let word = &text[words[i].0.clone()];
            let digits: String = word.chars().take_while(|c| is_numeral(*c)).collect();
            let rest = &word[digits.len()..];
            if !digits.is_empty()
                && COUNTERS.contains(&rest)
                && !IDIOMS.contains(&word)
                && let Some(n) = digits_or_value(&digits)
            {
                out.push_str(&text[pos..start]);
                out.push_str(&n);
                out.push_str(rest);
                pos = words[i].0.end;
            }
            i += 1;
            continue;
        }
        let mut end = words[j - 1].0.end;
        let mut next = words.get(j).map(|w| &text[w.0.clone()]);
        // 数の語のあとの、数で始まる 1 語（「三」「十分」）は、続きの数と数える言葉（三十分）。言い回しではない
        if let Some(word) = next {
            let digits: String = word.chars().take_while(|c| is_numeral(*c)).collect();
            if !digits.is_empty() && COUNTERS.contains(&&word[digits.len()..]) {
                end += digits.len();
                next = Some(&word[digits.len()..]);
            }
        }
        let number = &text[start..end];
        let before = i.checked_sub(1).map(|k| &text[words[k].0.clone()]);
        // 「2対1」の「1」は、前の「対」で数とわかる
        let idiom = words
            .get(j)
            .is_some_and(|w| IDIOMS.contains(&&text[start..w.0.end]));
        let counted = next.is_some_and(|w| COUNTERS.contains(&w) && !idiom)
            || before.is_some_and(|w| BEFORE.contains(&w) || w == "対");
        if counted && let Some(n) = digits_or_value(number) {
            out.push_str(&text[pos..start]);
            out.push_str(&n);
            pos = end;
        }
        // 数える言葉の 1 語に入り込んだ数（「十分」の「十」）を読んだら、その語は終わっている
        i = if end > words[j - 1].0.end { j + 1 } else { j };
    }
    out.push_str(&text[pos..]);
    out
}

/// 書き直した数。算用数字だけの数は、値を読み直さない（「0.05秒」の「05」を 5 にしない）。全角は半角にするだけ
fn digits_or_value(number: &str) -> Option<String> {
    let ascii = |c: char| match c {
        '0'..='9' => Some(c),
        '０'..='９' => char::from_u32(c as u32 - '０' as u32 + '0' as u32),
        _ => None,
    };
    match number.chars().map(ascii).collect::<Option<String>>() {
        Some(digits) => Some(digits),
        None => value(number).map(format_number),
    }
}

/// 漢数字（「三十五」「一万五千」「二〇二六」）と算用数字の混じった数（「1万5000」）の値
fn value(number: &str) -> Option<u64> {
    let digit = |c: char| match c {
        '0'..='9' => Some(c as u64 - '0' as u64),
        '０'..='９' => Some(c as u64 - '０' as u64),
        '〇' | '零' => Some(0),
        '一' => Some(1),
        '二' => Some(2),
        '三' => Some(3),
        '四' => Some(4),
        '五' => Some(5),
        '六' => Some(6),
        '七' => Some(7),
        '八' => Some(8),
        '九' => Some(9),
        _ => None,
    };
    // 位の字がなければ、1 字ずつの並び（「二〇二六」「35」）
    if number.chars().all(|c| digit(c).is_some()) {
        return number
            .chars()
            .try_fold(0u64, |n, c| n.checked_mul(10)?.checked_add(digit(c)?));
    }
    let (mut total, mut section, mut run) = (0u64, 0u64, None::<u64>);
    for c in number.chars() {
        if let Some(d) = digit(c) {
            run = Some(run.unwrap_or(0).checked_mul(10)?.checked_add(d)?);
            continue;
        }
        match c {
            '十' | '百' | '千' => {
                let unit = match c {
                    '十' => 10,
                    '百' => 100,
                    _ => 1000,
                };
                section = section.checked_add(run.take().unwrap_or(1).checked_mul(unit)?)?;
            }
            '万' | '億' => {
                let unit = if c == '万' { 10_000 } else { 100_000_000 };
                let part = section.checked_add(run.take().unwrap_or(0))?;
                total = total.checked_add(part.max(1).checked_mul(unit)?)?;
                section = 0;
            }
            _ => return None,
        }
    }
    total.checked_add(section)?.checked_add(run.unwrap_or(0))
}

/// 1 万より大きい数は、万と億を残す（「1万5000」「2億」）
fn format_number(n: u64) -> String {
    if n < 10_000 {
        return n.to_string();
    }
    let (oku, man, rest) = (n / 100_000_000, n / 10_000 % 10_000, n % 10_000);
    let mut out = String::new();
    if oku > 0 {
        out += &format!("{oku}億");
    }
    if man > 0 {
        out += &format!("{man}万");
    }
    if rest > 0 {
        out += &rest.to_string();
    }
    out
}

/// 言い切りの形。この語で話の間が来たら「。」にする
const ENDINGS: &[&str] = &[
    "です",
    "ます",
    "でした",
    "ました",
    "ません",
    "でしょ",
    "でしょう",
    "ね",
    "よ",
    "よね",
    "な",
    "わ",
    "ぞ",
    "ぜ",
    "さ",
    "かな",
    "じゃん",
    "っす",
    "か",
    "ねえ",
    "なあ",
];

/// `words[from..=last]` が言い切りで終わる文か。述語（動詞、形容詞、助動詞）があり、助詞か助動詞で、言い切りの形に終わる。
/// 「みんな」の「な」（名詞の一部）と、「えーね」（述語がない）は、言い切りではない
fn ends(text: &str, words: &[(std::ops::Range<usize>, String)], from: usize, last: usize) -> bool {
    let predicate = words[from..=last]
        .iter()
        .any(|w| matches!(pos(&w.1), "動詞" | "形容詞" | "助動詞"));
    let tail = matches!(pos(&words[last].1), "助詞" | "助動詞");
    let before = &text[..words[last].0.end];
    predicate && tail && ENDINGS.iter().any(|e| before.ends_with(e))
}

fn pos(feature: &str) -> &str {
    feature.split(',').next().unwrap_or_default()
}

/// 句読点のない文に、句読点を付ける。句読点がもうある文（会社の文字起こし、Haas-ja、Whisper）は変えない
fn punctuate(furigana: &Furigana, text: &str) -> String {
    let text = text.trim();
    if text.is_empty() || text.contains(['、', '。', '！', '？', '!', '?', '，', '．']) {
        return text.to_owned();
    }
    // 空白（話の間）の位置を覚えて、空白を除いた文を語に分ける
    let mut joined = String::new();
    let mut pauses = Vec::new();
    for (k, part) in text
        .split([' ', '　'])
        .filter(|p| !p.is_empty())
        .enumerate()
    {
        // 英字どうしの間（「Bullet Dance」）は語の区切りなので、空白のまま残す
        let latin = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
        if k > 0 && latin(joined.chars().last()) && latin(part.chars().next()) {
            joined.push(' ');
        } else if k > 0 {
            pauses.push(joined.len());
        }
        joined.push_str(part);
    }
    let words = furigana.words(&joined);
    let mut marks: Vec<(usize, char)> = Vec::new();
    // 今の区切りの最初の語
    let mut from = 0;
    for &at in &pauses {
        let prev = words.iter().position(|w| w.0.end == at);
        let next = words.iter().position(|w| w.0.start == at);
        // 語の途中の空白
        let (Some(prev), Some(next)) = (prev, next) else {
            continue;
        };
        // 名詞と名詞の間（「デッドロック 実況」）も、語の区切りに入った空白
        let (before, after) = (pos(&words[prev].1), pos(&words[next].1));
        // 「は」のあとの間（「キャラは ヘイズ」）は、字幕では区切らないことが多い
        let topic = matches!(&joined[words[prev].0.clone()], "は" | "って");
        let attached = matches!(after, "助詞" | "助動詞" | "接尾辞")
            || before == "接頭辞"
            || (before == "名詞" && after == "名詞")
            || topic;
        if attached {
            continue;
        }
        marks.push((
            at,
            if ends(&joined, &words, from, prev) {
                '。'
            } else {
                '、'
            },
        ));
        from = next;
    }
    // 文の初めの感動詞（「うわ」「よし」「えーと」）のあと
    // （すぐ後ろが助詞の「えーね」は、区切らない）
    if let Some(first) = words.first()
        && pos(&first.1) == "感動詞"
        && words
            .get(1)
            .is_some_and(|w| !matches!(pos(&w.1), "助詞" | "助動詞"))
        && !marks.iter().any(|(at, _)| *at == first.0.end)
    {
        marks.push((first.0.end, '、'));
    }
    marks.sort();
    let mut out = String::new();
    let mut from = 0;
    for (at, mark) in marks {
        out.push_str(&joined[from..at]);
        out.push(mark);
        from = at;
    }
    out.push_str(&joined[from..]);
    out.push('。');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn furigana() -> &'static Furigana {
        Furigana::shared().expect("dictionary")
    }

    #[test]
    fn counted_numbers_become_digits() {
        let f = furigana();
        assert_eq!(
            numbers(f, "三十五秒で一万五千ソウル"),
            "35秒で1万5000ソウル"
        );
        assert_eq!(numbers(f, "ヘイズの二番で三キル"), "ヘイズの2番で3キル");
        assert_eq!(numbers(f, "二対一は無理"), "2対1は無理");
        assert_eq!(numbers(f, "レベル五になった"), "レベル5になった");
        assert_eq!(numbers(f, "あと三十分後"), "あと30分後");
        // 算用数字は、値を読み直さない
        assert_eq!(numbers(f, "0.05秒だけ遅い"), "0.05秒だけ遅い");
        assert_eq!(numbers(f, "００７番"), "007番");
    }

    #[test]
    fn idioms_and_bare_numbers_stay() {
        let f = furigana();
        for text in [
            "一気に詰めた",
            "一緒に行こう",
            "一瞬で溶けた",
            "これが一番強い",
            "一人で来た",
            "これで十分強い",
            "一度やってみる",
        ] {
            assert_eq!(numbers(f, text), text);
        }
    }

    #[test]
    fn values_are_read_like_japanese() {
        assert_eq!(value("三十五"), Some(35));
        assert_eq!(value("一万五千"), Some(15_000));
        assert_eq!(value("二〇二六"), Some(2026));
        assert_eq!(value("1万5000"), Some(15_000));
        assert_eq!(value("百"), Some(100));
        assert_eq!(value("十万"), Some(100_000));
        assert_eq!(value("九九九九九九九九九九九九九九九九九九九九九九"), None);
        assert_eq!(value("九九九九九九九九九九九九九九九九万"), None);
        assert_eq!(format_number(15_000), "1万5000");
        assert_eq!(format_number(20_000), "2万");
        assert_eq!(format_number(123_456_789), "1億2345万6789");
    }

    #[test]
    fn pauses_become_commas_or_periods() {
        let f = furigana();
        assert_eq!(
            punctuate(f, "さあ始まりました デッドロック 実況です えーね 僕の"),
            "さあ、始まりました。デッドロック実況です。えーね、僕の。"
        );
        // 助詞の前の空白は、ただ消す
        assert_eq!(punctuate(f, "カース は最終盤"), "カースは最終盤。");
        assert_eq!(punctuate(f, "うわ痛い"), "うわ、痛い。");
        // 英字の語の間の空白は残す
        assert_eq!(punctuate(f, "Bullet Dance 強い"), "Bullet Dance、強い。");
        // 句読点のある文は変えない
        assert_eq!(
            punctuate(f, "もう書いてある、これ。"),
            "もう書いてある、これ。"
        );
    }
}
