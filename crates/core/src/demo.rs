//! ダミーの Segment を partial → final → enriched と流す（動作確認用）。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::sleep;

use crate::store::Store;
use crate::types::{Enriched, EnrichedKind, Segment, Stage, Token};

/// `[漢字|かんじ]` はルビ、`{hero:13|ヘイズ}` はエンティティ（`|` の後は発話された表現。
/// 訳文では辞書がまだないときに出す名前）。訳文は設定の翻訳先にある言語だけを流す
struct Line {
    source: &'static str,
    en: &'static str,
    ru: &'static str,
}

const LINES: &[Line] = &[
    Line {
        source: "さあ、[始|はじ]まりました。デッドロック[実況|じっきょう]です。",
        en: "Here we go! This is a Deadlock stream.",
        ru: "Итак, начинаем! Это стрим по Deadlock.",
    },
    Line {
        source: "[僕|ぼく]の[一番|いちばん][好|す]きなキャラは{hero:13|ヘイズ}ですね。",
        en: "My favorite character is {hero:13|Haze}.",
        ru: "Мой любимый персонаж — {hero:13|Haze}.",
    },
    Line {
        source: "{hero:13|ヘイズ}の{ability:731943444|バレットダンス}はめっちゃ[強|つよ]い。",
        en: "{hero:13|Haze}'s {ability:731943444|Bullet Dance} is really strong.",
        ru: "{ability:731943444|Bullet Dance} у {hero:13|Haze} очень сильный.",
    },
    Line {
        source: "まずは{item:1548066885|拡張マガジン}を[買|か]おう。",
        en: "Let's buy {item:1548066885|Extended Magazine} first.",
        ru: "Сначала купим {item:1548066885|Extended Magazine}.",
    },
];

static RUNNING: AtomicBool = AtomicBool::new(false);

/// デモを 1 周流す。すでに流れていれば何もしない
pub fn spawn_once(store: Arc<Store>) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        run(&store).await;
        RUNNING.store(false, Ordering::SeqCst);
    });
}

pub async fn run(store: &Store) {
    for (i, line) in LINES.iter().enumerate() {
        play_line(store, line).await;
        if i == 0 {
            play_noise(store).await;
        }
        sleep(Duration::from_millis(1500)).await;
    }
}

async fn play_line(store: &Store, line: &Line) {
    let source = parse(line.source);
    let final_text = plain(&source);

    let mut seg = new_segment();
    // realtime の partial は句読点なしで少しずつ伸びる
    let partial: Vec<char> = final_text
        .chars()
        .filter(|c| !"、。".contains(*c))
        .collect();
    for end in (2..=partial.len()).step_by(2).chain([partial.len()]) {
        seg.text = partial[..end].iter().collect();
        store.upsert_segment(seg.clone());
        sleep(Duration::from_millis(120)).await;
    }

    sleep(Duration::from_millis(800)).await;
    seg.stage = Stage::Final;
    seg.text = final_text;
    store.upsert_segment(seg.clone());

    sleep(Duration::from_millis(2000)).await;
    seg.stage = Stage::Enriched;
    seg.enriched = Some(Enriched {
        kind: EnrichedKind::Speech,
        source,
        translations: [("en", line.en), ("ru", line.ru)]
            .into_iter()
            .filter(|(lang, _)| store.settings().translate_to.iter().any(|t| t == lang))
            .map(|(lang, text)| (lang.into(), parse(text)))
            .collect(),
    });
    seg.settled_at = Some(now_ms());
    store.upsert_segment(seg);
}

/// 咳や「えー」などの断片が discarded で消えることを確かめる
async fn play_noise(store: &Store) {
    let mut seg = new_segment();
    seg.text = "えー".into();
    store.upsert_segment(seg.clone());
    sleep(Duration::from_millis(1200)).await;
    seg.stage = Stage::Discarded;
    store.upsert_segment(seg);
}

fn parse(markup: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut rest = markup;
    while let Some(start) = rest.find(['[', '{']) {
        if start > 0 {
            tokens.push(text(&rest[..start], None));
        }
        let close = if rest[start..].starts_with('[') {
            ']'
        } else {
            '}'
        };
        let end = start + rest[start..].find(close).expect("unclosed markup");
        let inner = &rest[start + 1..end];
        let (head, tail) = inner
            .split_once('|')
            .map_or((inner, None), |(h, t)| (h, Some(t)));
        tokens.push(if close == ']' {
            text(head, tail)
        } else {
            Token::Entity {
                reference: head.into(),
                spoken: tail.map(Into::into),
            }
        });
        rest = &rest[end + 1..];
    }
    if !rest.is_empty() {
        tokens.push(text(rest, None));
    }
    tokens
}

/// 音声認識の結果に相当する文字列（エンティティは発話された表現）
fn plain(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|t| match t {
            Token::Text { text, .. } => text.as_str(),
            Token::Entity { spoken, reference } => spoken.as_deref().unwrap_or(reference),
        })
        .collect()
}

fn text(text: &str, ruby: Option<&str>) -> Token {
    Token::Text {
        text: text.into(),
        ruby: ruby.map(Into::into),
    }
}

fn new_segment() -> Segment {
    let started_at = now_ms();
    Segment {
        id: format!("demo-{started_at}"),
        started_at,
        stage: Stage::Partial,
        lang: "ja".into(),
        text: String::new(),
        enriched: None,
        settled_at: None,
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ruby_and_entities() {
        let tokens = parse("[僕|ぼく]は{hero:13|ヘイズ}と{item:1}。");
        assert_eq!(
            tokens,
            vec![
                text("僕", Some("ぼく")),
                text("は", None),
                Token::Entity {
                    reference: "hero:13".into(),
                    spoken: Some("ヘイズ".into())
                },
                text("と", None),
                Token::Entity {
                    reference: "item:1".into(),
                    spoken: None
                },
                text("。", None),
            ]
        );
        assert_eq!(plain(&tokens), "僕はヘイズとitem:1。");
    }

    #[test]
    fn every_line_parses() {
        for line in LINES {
            for markup in [line.source, line.en, line.ru] {
                assert!(!parse(markup).is_empty());
            }
        }
    }
}
