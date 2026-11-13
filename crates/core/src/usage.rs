//! OpenAI の使用量と概算の料金（DESIGN.md 13 章）。
//!
//! 音声は送った長さ、LLM は応答の `usage` のトークン数を数え、モデルの単価を掛ける。
//! 今回（アプリを起動してから）と月ごと（UTC）に足し、月ごとの分は `usage.json` に保存する。
//! 届かなかった応答（タイムアウトなど）の分は数えられないので、請求より少なめに出ることがある。
//! 単価は OpenAI の料金ページ（2026-11）の値で、請求額とは少しずれる（概算）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use ts_rs::TS;

/// 何に使ったか
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Use {
    /// 話している途中の字幕（GPT-Live-Transcribe）
    Realtime,
    /// 確定の字幕（GPT-Transcribe）
    Batch,
    Translate,
    Furigana,
    /// 辞書の漢字名の読み
    Readings,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Amount {
    /// 送った音声の長さ（秒）
    pub seconds: f64,
    #[ts(type = "number")]
    pub tokens: u64,
    pub usd: f64,
    /// 単価の分からないモデルを使った（その分は usd に入っていない）
    pub unpriced: bool,
}

pub type Usage = BTreeMap<Use, Amount>;

/// 変わってから保存するまで
const SAVE_DELAY: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UsageView {
    /// アプリを起動してから
    pub session: Usage,
    /// 今月（UTC）
    pub month: Usage,
}

/// LLM の応答の `usage`
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tokens {
    /// キャッシュから読んだ分を含む
    pub input: u64,
    pub cached: u64,
    pub output: u64,
}

impl Tokens {
    pub fn from_response(usage: &serde_json::Value) -> Self {
        let n = |v: &serde_json::Value| v.as_u64().unwrap_or(0);
        Self {
            input: n(&usage["input_tokens"]),
            cached: n(&usage["input_tokens_details"]["cached_tokens"]),
            output: n(&usage["output_tokens"]),
        }
    }
}

enum Price {
    PerMinute(f64),
    /// 100 万トークンあたり
    PerMillion {
        input: f64,
        cached: f64,
        output: f64,
    },
}

fn price(model: &str) -> Option<Price> {
    Some(match model {
        "gpt-live-transcribe" => Price::PerMinute(0.017),
        "gpt-transcribe" => Price::PerMinute(0.0045),
        // xAI の REST は 1 時間 $0.10
        "grok-voice-transcribe-2.0" => Price::PerMinute(0.10 / 60.0),
        // QwenCloud シンガポール。音声 1 秒が入力約 11 トークン（料金の比較サイトの値。DESIGN.md 3.4）
        "qwen-audio-3.1-asr-flash" => Price::PerMillion {
            input: 0.15,
            cached: 0.15,
            output: 0.47,
        },
        "qwen-audio-3.1-asr-flash-streaming" => Price::PerMillion {
            input: 0.93,
            cached: 0.93,
            output: 0.70,
        },
        // OpenRouter でも 1 時間 $0.10
        "microsoft/mai-transcribe-2" => Price::PerMinute(0.10 / 60.0),
        // PC の中で動くので無料
        crate::local::HAAS_JA => Price::PerMinute(0.0),
        crate::llama::GEMMA => Price::PerMillion {
            input: 0.0,
            cached: 0.0,
            output: 0.0,
        },
        "gpt-6.1-terra" => Price::PerMillion {
            input: 1.00,
            cached: 0.05,
            output: 5.00,
        },
        // Decisions API も同じ入力の単価で、出力は数えられない
        "gpt-6-luna" => Price::PerMillion {
            input: 0.10,
            cached: 0.01,
            output: 0.50,
        },
        // 料金の比較サイトの値（QwenCloud の料金ページでは確かめていない。DESIGN.md 9.7）
        "qwen-mt-flash" => Price::PerMillion {
            input: 0.16,
            cached: 0.16,
            output: 0.49,
        },
        // TypeSafe AI の Jev。10 億トークン $42 で、出力は数えられない（解説の記事の値。DESIGN.md 9.7）
        "jev-latest" => Price::PerMillion {
            input: 0.042,
            cached: 0.042,
            output: 0.0,
        },
        _ => return None,
    })
}

/// 月ごとの分（保存する形）
#[derive(Default, Serialize, Deserialize)]
struct Months {
    /// "2026-11" → 使用量
    months: BTreeMap<String, Usage>,
}

pub struct Meter {
    path: PathBuf,
    /// 今回の分と、月ごとの分
    state: Mutex<(Usage, Months)>,
    /// 画面に出す最新の値。音声を送るたびに変わるので、字幕などと同じ broadcast には流さない
    view: watch::Sender<UsageView>,
}

impl Meter {
    /// `path` が読めなければ今月の分を 0 から数える（壊れていても止めない）
    pub fn new(path: PathBuf) -> Self {
        let months: Months = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let view = UsageView {
            session: Usage::new(),
            month: months
                .months
                .get(&month_key(SystemTime::now()))
                .cloned()
                .unwrap_or_default(),
        };
        Self {
            path,
            state: Mutex::new((Usage::new(), months)),
            view: watch::Sender::new(view),
        }
    }

    pub fn audio(&self, what: Use, model: &str, seconds: f64) {
        let usd = match price(model) {
            Some(Price::PerMinute(per_minute)) => Some(seconds / 60.0 * per_minute),
            _ => None,
        };
        self.add(
            what,
            Amount {
                seconds,
                usd: usd.unwrap_or(0.0),
                unpriced: usd.is_none(),
                ..Amount::default()
            },
        );
    }

    pub fn tokens(&self, what: Use, model: &str, tokens: Tokens) {
        let usd = match price(model) {
            Some(Price::PerMillion {
                input,
                cached,
                output,
            }) => {
                let fresh = tokens.input.saturating_sub(tokens.cached);
                Some(
                    (fresh as f64 * input
                        + tokens.cached as f64 * cached
                        + tokens.output as f64 * output)
                        / 1_000_000.0,
                )
            }
            _ => None,
        };
        self.add(
            what,
            Amount {
                tokens: tokens.input + tokens.output,
                usd: usd.unwrap_or(0.0),
                unpriced: usd.is_none(),
                ..Amount::default()
            },
        );
    }

    /// 料金を答えに入れてくる会社（OpenRouter）の分
    pub fn billed(&self, what: Use, tokens: Tokens, usd: f64) {
        self.add(
            what,
            Amount {
                tokens: tokens.input + tokens.output,
                usd,
                ..Amount::default()
            },
        );
    }

    pub fn watch(&self) -> watch::Receiver<UsageView> {
        self.view.subscribe()
    }

    /// 変わったら、少し待ってまとめて保存する（音声を送るたびに書かない）
    pub async fn persist(&self) {
        let mut changed = self.view.subscribe();
        while changed.changed().await.is_ok() {
            tokio::time::sleep(SAVE_DELAY).await;
            changed.mark_unchanged();
            if let Err(e) = self.save().await {
                tracing::warn!("cannot save usage: {e:#}");
            }
        }
    }

    async fn save(&self) -> anyhow::Result<()> {
        let json = serde_json::to_vec_pretty(&self.state.lock().unwrap().1)?;
        crate::write_atomic(&self.path, &json).await
    }

    fn add(&self, what: Use, amount: Amount) {
        let (session, months) = &mut *self.state.lock().unwrap();
        add(session.entry(what).or_default(), &amount);
        let month = months
            .months
            .entry(month_key(SystemTime::now()))
            .or_default();
        add(month.entry(what).or_default(), &amount);
        self.view.send_replace(UsageView {
            session: session.clone(),
            month: month.clone(),
        });
    }
}

fn add(total: &mut Amount, amount: &Amount) {
    total.seconds += amount.seconds;
    total.tokens += amount.tokens;
    total.usd += amount.usd;
    total.unpriced |= amount.unpriced;
}

/// UTC の年月（"2026-11"）。OpenAI の請求も UTC の月ごと
fn month_key(now: SystemTime) -> String {
    let days = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() / 86_400;
    // 1970-01-01 からの日数を暦に直す（Howard Hinnant の civil_from_days）
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year}-{month:02}")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn meter(dir: &tempfile::TempDir) -> Meter {
        Meter::new(dir.path().join("usage.json"))
    }

    #[test]
    fn month_key_is_utc_year_and_month() {
        let at = |secs| month_key(UNIX_EPOCH + Duration::from_secs(secs));
        assert_eq!(at(0), "1970-01");
        // 2026-11-03T00:00:00Z
        assert_eq!(at(1_793_664_000), "2026-11");
        // 2024-02-29T23:59:59Z（うるう日）と、その次の秒
        assert_eq!(at(1_709_251_199), "2024-02");
        assert_eq!(at(1_709_251_200), "2024-03");
    }

    #[test]
    fn prices_audio_by_minute_and_tokens_by_million() {
        let dir = tempfile::tempdir().unwrap();
        let m = meter(&dir);
        m.audio(Use::Realtime, "gpt-live-transcribe", 120.0);
        m.tokens(
            Use::Translate,
            "gpt-6.1-terra",
            Tokens {
                input: 1_000,
                cached: 400,
                output: 200,
            },
        );
        let view = m.watch().borrow().clone();
        let realtime = &view.session[&Use::Realtime];
        assert!((realtime.usd - 0.034).abs() < 1e-9);
        assert_eq!(realtime.seconds, 120.0);
        // 600 × $1 + 400 × $0.05 + 200 × $5（100 万トークンあたり）
        let translate = &view.session[&Use::Translate];
        assert!((translate.usd - 0.00162).abs() < 1e-9);
        assert_eq!(translate.tokens, 1_200);
        assert_eq!(view.month, view.session);
    }

    #[test]
    fn unknown_model_is_marked_unpriced() {
        let dir = tempfile::tempdir().unwrap();
        let m = meter(&dir);
        m.audio(Use::Batch, "some-new-model", 30.0);
        let view = m.watch().borrow().clone();
        let batch = &view.session[&Use::Batch];
        assert!(batch.unpriced);
        assert_eq!(batch.usd, 0.0);
        assert_eq!(batch.seconds, 30.0);
    }

    #[tokio::test]
    async fn month_survives_restart_but_session_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let m = meter(&dir);
        m.audio(Use::Batch, "gpt-transcribe", 60.0);
        m.save().await.unwrap();
        let view = meter(&dir).watch().borrow().clone();
        assert!(view.session.is_empty());
        assert_eq!(view.month[&Use::Batch].seconds, 60.0);
    }

    #[test]
    fn response_usage_counts_cached_tokens() {
        let usage = serde_json::json!({
            "input_tokens": 900, "output_tokens": 50,
            "input_tokens_details": { "cached_tokens": 512 },
        });
        assert_eq!(
            Tokens::from_response(&usage),
            Tokens {
                input: 900,
                cached: 512,
                output: 50
            }
        );
    }
}
