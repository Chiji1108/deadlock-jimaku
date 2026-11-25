//! 「アプリを起動してから」と「今月（UTC）」に足していく記録。月ごとの分はファイルに保存する。
//! 使用量（`usage`）が使う。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use ts_rs::TS;

/// 変わってから保存するまで
const SAVE_DELAY: Duration = Duration::from_secs(5);

/// 画面に出す 2 つの期間
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Periods<T> {
    /// アプリを起動してから
    pub session: T,
    /// 今月（UTC）
    pub month: T,
}

/// 月ごとの分（保存する形）
#[derive(Default, Serialize, Deserialize)]
#[serde(bound = "T: Serialize + DeserializeOwned")]
struct Months<T> {
    /// "2026-11" → その月の分
    months: BTreeMap<String, T>,
}

pub struct Monthly<T> {
    path: PathBuf,
    /// 今回の分と、月ごとの分
    state: Mutex<(T, Months<T>)>,
    /// 画面に出す最新の値。頻繁に変わるので、字幕などと同じ broadcast には流さない
    view: watch::Sender<Periods<T>>,
}

impl<T: Clone + Default + Serialize + DeserializeOwned> Monthly<T> {
    /// `path` が読めなければ今月の分を 0 から数える（壊れていても止めない）
    pub fn new(path: PathBuf) -> Self {
        let months: Months<T> = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let view = Periods {
            session: T::default(),
            month: months
                .months
                .get(&month_key(SystemTime::now()))
                .cloned()
                .unwrap_or_default(),
        };
        Self {
            path,
            state: Mutex::new((T::default(), months)),
            view: watch::Sender::new(view),
        }
    }

    /// 今回の分と今月の分に、同じように足す
    pub fn add(&self, add: impl Fn(&mut T)) {
        let (session, months) = &mut *self.state.lock().unwrap();
        add(session);
        let month = months
            .months
            .entry(month_key(SystemTime::now()))
            .or_default();
        add(month);
        self.view.send_replace(Periods {
            session: session.clone(),
            month: month.clone(),
        });
    }

    pub fn watch(&self) -> watch::Receiver<Periods<T>> {
        self.view.subscribe()
    }

    /// 変わったら、少し待ってまとめて保存する（足すたびに書かない）
    pub async fn persist(&self) {
        let mut changed = self.view.subscribe();
        while changed.changed().await.is_ok() {
            tokio::time::sleep(SAVE_DELAY).await;
            changed.mark_unchanged();
            if let Err(e) = self.save().await {
                tracing::warn!("cannot save {}: {e:#}", self.path.display());
            }
        }
    }

    pub(crate) async fn save(&self) -> anyhow::Result<()> {
        let json = serde_json::to_vec_pretty(&self.state.lock().unwrap().1)?;
        crate::write_atomic(&self.path, &json).await
    }
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
    use super::*;

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

    #[tokio::test]
    async fn month_survives_restart_but_session_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.json");
        let n = Monthly::<u64>::new(path.clone());
        n.add(|n| *n += 3);
        n.save().await.unwrap();
        let view = Monthly::<u64>::new(path).watch().borrow().clone();
        assert_eq!(
            view,
            Periods {
                session: 0,
                month: 3
            }
        );
    }
}
