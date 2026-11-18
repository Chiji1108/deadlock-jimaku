//! ダミーの Segment を partial → final → enriched と流す（動作確認用）。
//! 「おすすめ」のモードと同じく、話している途中からふりがな・用語・訳を付ける。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::time::sleep;

use crate::markup::{parse, plain};
use crate::now_ms;
use crate::store::Store;
use crate::types::{Enriched, Segment, Stage, Token, Translations};

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
        source: "[私|わたし]の[一番|いちばん][好|す]きなキャラは{hero:13|ヘイズ}ですね。",
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

/// 途中の訳が、話した文字より何文字ぶん遅れるか
const DRAFT_LAG: usize = 4;

async fn play_line(store: &Store, line: &Line) {
    let source = parse(line.source);
    let final_text = plain(&source);
    let translations: Vec<(String, Vec<Token>)> = [("en", line.en), ("ru", line.ru)]
        .into_iter()
        .filter(|(lang, _)| store.settings().translate_to.iter().any(|t| t == lang))
        .map(|(lang, text)| (lang.into(), parse(text)))
        .collect();

    let mut seg = new_segment();
    let total = final_text.chars().count();
    for end in (2..=total).step_by(2).chain([total]) {
        seg.text = final_text.chars().take(end).collect();
        seg.tokens = Some(revealed(&source, end, false));
        // 訳は、話した分だけ、少し遅れて進む
        let spoken = if end == total {
            total
        } else {
            end.saturating_sub(DRAFT_LAG)
        };
        let draft = translations.iter().filter_map(|(lang, tokens)| {
            let shown = revealed(tokens, length(tokens) * spoken / total, true);
            (!shown.is_empty()).then(|| (lang.clone(), shown))
        });
        seg.draft = Some(draft.collect()).filter(|draft: &Translations| !draft.is_empty());
        store.upsert_segment(seg.clone());
        sleep(Duration::from_millis(120)).await;
    }

    sleep(Duration::from_millis(800)).await;
    seg.stage = Stage::Final;
    store.upsert_segment(seg.clone());

    sleep(Duration::from_millis(2000)).await;
    seg.stage = Stage::Enriched;
    seg.draft = None;
    seg.enriched = Some(Enriched {
        source,
        translations: translations.into_iter().collect(),
    });
    seg.settled_at = Some(now_ms());
    store.upsert_segment(seg);
}

/// トークンの、字幕に出る文字
fn written(token: &Token) -> &str {
    match token {
        Token::Text { text, .. } => text,
        Token::Entity { spoken, .. } => spoken.as_deref().unwrap_or_default(),
    }
}

fn length(tokens: &[Token]) -> usize {
    tokens.iter().map(|t| written(t).chars().count()).sum()
}

/// 初めの `count` 文字までのトークン。途中で切れた語はただの文字にする。`words` なら、語の途中では切らない
fn revealed(tokens: &[Token], count: usize, words: bool) -> Vec<Token> {
    let mut rest = count;
    let mut shown = Vec::new();
    for token in tokens {
        if rest == 0 {
            break;
        }
        let text = written(token);
        let len = text.chars().count();
        if len <= rest {
            shown.push(token.clone());
            rest -= len;
            continue;
        }
        let cut: String = text.chars().take(rest).collect();
        // 訳では、語の途中で切らない。用語の名前は、全部の字が出るまで出さない
        let cut = match (words, token, cut.rfind(' ')) {
            (false, ..) => cut,
            (true, Token::Text { .. }, Some(space)) => cut[..=space].to_owned(),
            (true, ..) => String::new(),
        };
        if !cut.is_empty() {
            shown.push(Token::Text {
                text: cut,
                ruby: None,
            });
        }
        break;
    }
    shown
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

fn new_segment() -> Segment {
    let started_at = now_ms();
    Segment {
        id: format!("demo-{started_at}"),
        started_at,
        stage: Stage::Partial,
        lang: "ja".into(),
        text: String::new(),
        enriched: None,
        tokens: None,
        draft: None,
        settled_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_parses() {
        for line in LINES {
            for markup in [line.source, line.en, line.ru] {
                assert!(!parse(markup).is_empty());
            }
        }
    }

    #[test]
    fn a_draft_grows_by_words_and_keeps_whole_terms() {
        let tokens = parse("{hero:13|Haze}'s {ability:1|Bullet Dance} is strong.");
        let text = |count| plain(&revealed(&tokens, count, true));
        assert_eq!(text(0), "");
        assert_eq!(text(4), "Haze");
        // 用語は、全部の字が出るまで出さない
        assert_eq!(text(8), "Haze's ");
        assert_eq!(text(14), "Haze's ");
        assert_eq!(text(19), "Haze's Bullet Dance");
        assert_eq!(text(24), "Haze's Bullet Dance is ");
        assert_eq!(revealed(&tokens, length(&tokens), true), tokens);
        // 話した文は、字で切る
        let source = parse("[強|つよ]い{hero:13|ヘイズ}");
        assert_eq!(plain(&revealed(&source, 3, false)), "強いヘ");
    }
}
