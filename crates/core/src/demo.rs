//! ダミーの Segment を partial → final → enriched と流す（Phase 0 の動作確認用）。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::sleep;

use crate::store::Store;
use crate::types::{Enriched, EnrichedKind, Segment, Stage, Token};

struct Line {
    final_text: &'static str,
    /// (text, ruby) / エンティティは ruby の代わりに ref を `@` で始めて書く
    source: &'static [(&'static str, &'static str)],
    en: &'static str,
    ru: &'static str,
}

const LINES: &[Line] = &[
    Line {
        final_text: "さあ、始まりました。デッドロック実況です。",
        source: &[
            ("さあ、", ""),
            ("始", "はじ"),
            ("まりました。", ""),
            ("デッドロック", ""),
            ("実況", "じっきょう"),
            ("です。", ""),
        ],
        en: "Here we go! This is a Deadlock stream.",
        ru: "Итак, начинаем! Это стрим по Deadlock.",
    },
    Line {
        final_text: "僕の一番好きなキャラはヘイズですね。",
        source: &[
            ("僕", "ぼく"),
            ("の", ""),
            ("一番", "いちばん"),
            ("好", "す"),
            ("きなキャラは", ""),
            ("ヘイズ", "@hero:13"),
            ("ですね。", ""),
        ],
        en: "My favorite character is Haze.",
        ru: "Мой любимый персонаж — Haze.",
    },
    Line {
        final_text: "ヘイズのバレットダンスもめっちゃ強いんだけど。",
        source: &[
            ("ヘイズ", "@hero:13"),
            ("の", ""),
            ("バレットダンス", "@ability:bullet_dance"),
            ("もめっちゃ", ""),
            ("強", "つよ"),
            ("いんだけど。", ""),
        ],
        en: "Haze's Bullet Dance is really strong too, though.",
        ru: "Bullet Dance у Haze тоже очень сильный.",
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
    let mut seg = new_segment();
    // realtime の partial は句読点なしで少しずつ伸びる
    let partial: Vec<char> = line
        .final_text
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
    seg.text = line.final_text.into();
    store.upsert_segment(seg.clone());

    sleep(Duration::from_millis(2000)).await;
    seg.stage = Stage::Enriched;
    seg.enriched = Some(Enriched {
        kind: EnrichedKind::Speech,
        source: line
            .source
            .iter()
            .map(|&(text, ruby)| token(text, ruby))
            .collect(),
        translations: [("en", line.en), ("ru", line.ru)]
            .into_iter()
            .map(|(lang, text)| (lang.into(), vec![token(text, "")]))
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

fn token(text: &str, ruby: &str) -> Token {
    match ruby.strip_prefix('@') {
        Some(reference) => Token::Entity {
            reference: reference.into(),
            spoken: Some(text.into()),
        },
        None => Token::Text {
            text: text.into(),
            ruby: (!ruby.is_empty()).then(|| ruby.into()),
        },
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
