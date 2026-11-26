//! 字幕の統計。字幕を始めてから止めるまでを 1 回として、確定した字幕から、喋った量と、
//! 呼んだヒーロー・アイテム・スキルの回数を数える。文そのものは残さず、数だけを `stats.db`（SQLite）に書く。
//! 字幕ごとにも、話し始めた時刻・言語・数を 1 行残す（回の中の流れや、日付をまたいだ回を後から分けるため）。
//! 今週・今月・ぜんぶの合計は、見せるときに回ごとの数を足して出す（期間ごとの数は持たない）。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use ts_rs::TS;

use crate::now_ms;
use crate::types::{Segment, Token};

/// 数える用語の種類。人と基本用語は数えない
const COUNTED: &[&str] = &["hero:", "item:", "ability:"];
/// 画面に出す回の数（DB には全部残す）
const RECENT: u32 = 50;
/// テーブルの形を変えたら上げて、`migrate` に足す
const SCHEMA: i32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Totals {
    /// 確定した字幕の数
    #[ts(type = "number")]
    pub utterances: u64,
    /// 空白を除いた文字数（空白で区切らない言語で見せる）
    #[ts(type = "number")]
    pub characters: u64,
    /// 空白で区切った語の数（英語などで見せる）
    #[ts(type = "number")]
    pub words: u64,
    /// `"hero:13"` → 呼んだ回数。ヒーロー・アイテム・スキルだけ
    #[ts(type = "Record<string, number>")]
    pub names: BTreeMap<String, u64>,
}

/// 字幕を始めてから止めるまでの 1 回
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Session {
    #[ts(type = "number")]
    pub id: i64,
    /// 字幕を始めた時刻（epoch ms）
    #[ts(type = "number")]
    pub started_at: u64,
    /// 止めた時刻か、最後に字幕が確定した時刻（epoch ms）
    #[ts(type = "number")]
    pub ended_at: u64,
    /// 今、字幕を出している回
    pub live: bool,
    #[serde(flatten)]
    pub totals: Totals,
}

/// 今日・今週・今月・ぜんぶの、1 つの期間
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StatsPeriod {
    /// この時刻（epoch ms）以降に始めた回を足す。ぜんぶは 0
    #[ts(type = "number")]
    pub since: u64,
    /// 字幕を出していた時間の合計（ms。今出している回は、最後に字幕が確定したところまで）
    #[ts(type = "number")]
    pub duration_ms: u64,
    /// 字幕を出した日の数（PC の時刻で、回を始めた日）
    #[ts(type = "number")]
    pub active_days: u64,
    #[serde(flatten)]
    pub totals: Totals,
}

/// 1 日の数（その日に始めた回の合計）
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StatsDay {
    /// その日の 0 時（PC の時刻。epoch ms）
    #[ts(type = "number")]
    pub start: u64,
    #[ts(type = "number")]
    pub utterances: u64,
    #[ts(type = "number")]
    pub characters: u64,
    #[ts(type = "number")]
    pub words: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StatsView {
    /// 今週（月曜の 0 時から）
    pub week: StatsPeriod,
    /// 今月（1 日の 0 時から）
    pub month: StatsPeriod,
    /// すべての回
    pub all: StatsPeriod,
    /// 52 週前の月曜から、回のあった日だけ、古い順に（日ごとの草）
    pub days: Vec<StatsDay>,
    /// 直近 90 日に字幕を出していた時間（ms）。月曜 0 時から 1 時間ごとに 7 × 24 個（時間帯の表）
    #[ts(type = "number[]")]
    pub hours: Vec<u64>,
    /// 新しい順に、最近の回
    pub sessions: Vec<Session>,
}

pub struct Tally {
    db: Mutex<Db>,
    /// 画面に出す最新の値
    view: watch::Sender<StatsView>,
}

struct Db {
    conn: Connection,
    /// 字幕を出している
    running: bool,
    /// 数えている回。止めたあとに仕上がった字幕も、この回に数える
    current: Option<i64>,
    /// 始めたが、まだ字幕が確定していない回の、始めた時刻（字幕のない回は残さない）
    started_at: Option<u64>,
}

impl Tally {
    /// `path` を開けなければ、メモリの中だけで数える（壊れていても止めない）
    pub fn open(path: &Path) -> Self {
        // 初めて起動したときは、データの置き場所がまだない（設定を保存するまで作られない）
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            tracing::warn!("cannot create {}: {e:#}", dir.display());
        }
        let conn = Connection::open(path)
            .and_then(migrate)
            .unwrap_or_else(|e| {
                tracing::warn!("cannot open {}: {e:#}; stats are not saved", path.display());
                migrate(Connection::open_in_memory().expect("in-memory sqlite"))
                    .expect("in-memory schema")
            });
        let db = Db {
            conn,
            running: false,
            current: None,
            started_at: None,
        };
        let view = db.view().unwrap_or_default();
        Self {
            db: Mutex::new(db),
            view: watch::Sender::new(view),
        }
    }

    /// 字幕を出しているか。始めたとき・止めたときだけ回が変わる（設定を変えて処理を作り直しても同じ回）
    pub fn set_running(&self, running: bool) {
        let mut db = self.db.lock().unwrap();
        if db.running == running {
            return;
        }
        db.running = running;
        let now = now_ms();
        if running {
            db.current = None;
            db.started_at = Some(now);
        } else {
            db.started_at = None;
            if let Some(id) = db.current
                && let Err(e) = db.conn.execute(
                    "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
                    params![now, id],
                )
            {
                tracing::warn!("cannot end the stats session: {e:#}");
            }
        }
        self.publish(&db);
    }

    /// 確定した字幕を 1 つ数える
    pub fn count(&self, seg: &Segment) {
        let tokens = seg
            .enriched
            .as_ref()
            .map(|e| e.source.as_slice())
            .or(seg.tokens.as_deref())
            .unwrap_or_default();
        let names = tokens.iter().filter_map(|t| match t {
            Token::Entity { reference, .. }
                if COUNTED.iter().any(|kind| reference.starts_with(kind)) =>
            {
                Some(reference.as_str())
            }
            _ => None,
        });
        let characters = seg.text.chars().filter(|c| !c.is_whitespace()).count() as u64;
        let words = seg.text.split_whitespace().count() as u64;

        let mut db = self.db.lock().unwrap();
        if let Err(e) = db.add(seg.spoken_at, &seg.lang, names, characters, words) {
            tracing::warn!("cannot count the subtitle: {e:#}");
        }
        self.publish(&db);
    }

    /// 回を消す（`None` ならすべて）。名前と字幕ごとの行も一緒に消える。
    /// 字幕を出している回を消したら、続きは新しい回として数える
    pub fn forget(&self, session: Option<i64>) -> rusqlite::Result<()> {
        let mut db = self.db.lock().unwrap();
        match session {
            Some(id) => db
                .conn
                .execute("DELETE FROM sessions WHERE id = ?1", params![id])?,
            None => db.conn.execute("DELETE FROM sessions", [])?,
        };
        if db
            .current
            .is_some_and(|current| session.is_none_or(|id| id == current))
        {
            db.current = None;
            db.started_at = db.running.then(now_ms);
        }
        self.publish(&db);
        Ok(())
    }

    pub fn watch(&self) -> watch::Receiver<StatsView> {
        self.view.subscribe()
    }

    /// 今の値を数え直す（つながったときに送る。日付が変わっていても今日の値で出す）
    pub fn view(&self) -> StatsView {
        let db = self.db.lock().unwrap();
        self.publish(&db);
        self.view.borrow().clone()
    }

    fn publish(&self, db: &Db) {
        match db.view() {
            Ok(view) => {
                self.view.send_if_modified(|current| {
                    let changed = *current != view;
                    *current = view;
                    changed
                });
            }
            Err(e) => tracing::warn!("cannot read stats: {e:#}"),
        }
    }
}

impl Db {
    fn add<'a>(
        &mut self,
        spoken_at: u64,
        lang: &str,
        names: impl Iterator<Item = &'a str>,
        characters: u64,
        words: u64,
    ) -> rusqlite::Result<()> {
        let now = now_ms();
        let tx = self.conn.transaction()?;
        let id = match self.current {
            Some(id) => id,
            None => {
                let started_at = self.started_at.unwrap_or(now);
                tx.execute(
                    "INSERT INTO sessions (started_at, ended_at) VALUES (?1, ?1)",
                    params![started_at],
                )?;
                tx.last_insert_rowid()
            }
        };
        tx.execute(
            "UPDATE sessions SET utterances = utterances + 1, characters = characters + ?1,
                words = words + ?2, ended_at = MAX(ended_at, ?3) WHERE id = ?4",
            params![characters, words, now, id],
        )?;
        tx.execute(
            "INSERT INTO utterances (session_id, started_at, lang, characters, words)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, spoken_at, lang, characters, words],
        )?;
        for name in names {
            tx.execute(
                "INSERT INTO session_names (session_id, ref, count) VALUES (?1, ?2, 1)
                 ON CONFLICT (session_id, ref) DO UPDATE SET count = count + 1",
                params![id, name],
            )?;
        }
        tx.commit()?;
        self.current = Some(id);
        self.started_at = None;
        Ok(())
    }

    /// `since`（epoch ms）以降に始めた回の合計
    fn period(&self, since: i64) -> rusqlite::Result<StatsPeriod> {
        let mut period = self.conn.query_row(
            "SELECT COALESCE(SUM(utterances), 0), COALESCE(SUM(characters), 0),
                COALESCE(SUM(words), 0), COALESCE(SUM(ended_at - started_at), 0),
                COUNT(DISTINCT date(started_at / 1000, 'unixepoch', 'localtime'))
             FROM sessions WHERE started_at >= ?1",
            params![since],
            |row| {
                Ok(StatsPeriod {
                    since: since as u64,
                    duration_ms: row.get(3)?,
                    active_days: row.get(4)?,
                    totals: Totals {
                        utterances: row.get(0)?,
                        characters: row.get(1)?,
                        words: row.get(2)?,
                        names: BTreeMap::new(),
                    },
                })
            },
        )?;
        let mut names = self.conn.prepare_cached(
            "SELECT n.ref, SUM(n.count) FROM session_names n JOIN sessions s ON s.id = n.session_id
             WHERE s.started_at >= ?1 GROUP BY n.ref",
        )?;
        period.totals.names = names
            .query_map(params![since], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(period)
    }

    /// `since` 以降の、日ごとの数（PC の時刻で、回を始めた日に数える）
    fn days(&self, since: i64) -> rusqlite::Result<Vec<StatsDay>> {
        let mut days = self.conn.prepare_cached(
            "SELECT unixepoch(started_at / 1000, 'unixepoch', 'localtime', 'start of day', 'utc') * 1000
                    AS day,
                SUM(utterances), SUM(characters), SUM(words)
             FROM sessions WHERE started_at >= ?1 GROUP BY day ORDER BY day",
        )?;
        days.query_map(params![since], |row| {
            Ok(StatsDay {
                start: row.get(0)?,
                utterances: row.get(1)?,
                characters: row.get(2)?,
                words: row.get(3)?,
            })
        })?
        .collect()
    }

    /// `since` 以降に始めた回の時間を、PC の時刻の曜日と時に分ける（月曜 0 時から 7 × 24 個）。
    /// 回を 1 時間の区切りで切って、切れ端の長さを足す
    fn hours(&self, since: i64) -> rusqlite::Result<Vec<u64>> {
        let mut hours = vec![0; 7 * 24];
        let mut slices = self.conn.prepare_cached(
            "WITH RECURSIVE slices (start, stop) AS (
                SELECT started_at, ended_at FROM sessions
                WHERE started_at >= ?1 AND ended_at > started_at
                UNION ALL
                SELECT (start / 3600000 + 1) * 3600000, stop FROM slices
                WHERE (start / 3600000 + 1) * 3600000 < stop
            )
            SELECT (CAST(strftime('%w', start / 1000, 'unixepoch', 'localtime') AS INTEGER) + 6) % 7,
                CAST(strftime('%H', start / 1000, 'unixepoch', 'localtime') AS INTEGER),
                SUM(MIN(stop, (start / 3600000 + 1) * 3600000) - start)
            FROM slices GROUP BY 1, 2",
        )?;
        let rows = slices.query_map(params![since], |row| {
            Ok((
                row.get::<_, usize>(0)?,
                row.get::<_, usize>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })?;
        for row in rows {
            let (weekday, hour, ms) = row?;
            if let Some(slot) = hours.get_mut(weekday * 24 + hour) {
                *slot = ms;
            }
        }
        Ok(hours)
    }

    fn view(&self) -> rusqlite::Result<StatsView> {
        // PC の時刻で、今週（月曜）・今月の初め（epoch ms）。
        // 週は、6 日戻ってから次の月曜（その日が月曜ならその日）へ進める
        // 草は 52 週前の月曜から、時間帯は 90 日前の 0 時から
        let (week, month, year, recent): (i64, i64, i64, i64) = self.conn.query_row(
            "SELECT unixepoch('now', 'localtime', 'start of day', '-6 days', 'weekday 1', 'utc') * 1000,
                unixepoch('now', 'localtime', 'start of month', 'utc') * 1000,
                unixepoch('now', 'localtime', 'start of day', '-6 days', 'weekday 1', '-364 days', 'utc')
                    * 1000,
                unixepoch('now', 'localtime', 'start of day', '-89 days', 'utc') * 1000",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let (days, hours) = (self.days(year)?, self.hours(recent)?);
        let (week, month, all) = (self.period(week)?, self.period(month)?, self.period(0)?);

        let live = self.current.filter(|_| self.running);
        let mut recent = self.conn.prepare_cached(
            "SELECT id, started_at, ended_at, utterances, characters, words
             FROM sessions ORDER BY started_at DESC, id DESC LIMIT ?1",
        )?;
        let mut sessions: Vec<Session> = recent
            .query_map(params![RECENT], |row| {
                let id = row.get(0)?;
                Ok(Session {
                    id,
                    started_at: row.get(1)?,
                    ended_at: row.get(2)?,
                    live: live == Some(id),
                    totals: Totals {
                        utterances: row.get(3)?,
                        characters: row.get(4)?,
                        words: row.get(5)?,
                        names: BTreeMap::new(),
                    },
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut session_names = self
            .conn
            .prepare_cached("SELECT ref, count FROM session_names WHERE session_id = ?1")?;
        for session in &mut sessions {
            session.totals.names = session_names
                .query_map(params![session.id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
        }
        Ok(StatsView {
            week,
            month,
            all,
            days,
            hours,
            sessions,
        })
    }
}

fn migrate(conn: Connection) -> rusqlite::Result<Connection> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    // WAL なら、落ちても壊れない。書くたびにディスクを待たない
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    let version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < 1 {
        conn.execute_batch(
            "CREATE TABLE sessions (
                id INTEGER PRIMARY KEY,
                started_at INTEGER NOT NULL,
                ended_at INTEGER NOT NULL,
                utterances INTEGER NOT NULL DEFAULT 0,
                characters INTEGER NOT NULL DEFAULT 0,
                words INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX sessions_started_at ON sessions (started_at);
            CREATE TABLE session_names (
                session_id INTEGER NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
                ref TEXT NOT NULL,
                count INTEGER NOT NULL,
                PRIMARY KEY (session_id, ref)
            );
            -- 確定した字幕ごとに 1 行。started_at は話し始めた時刻（epoch ms）
            CREATE TABLE utterances (
                session_id INTEGER NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
                started_at INTEGER NOT NULL,
                lang TEXT NOT NULL,
                characters INTEGER NOT NULL,
                words INTEGER NOT NULL
            );
            CREATE INDEX utterances_session_id ON utterances (session_id, started_at);",
        )?;
    }
    // 新しい版で作った DB を古い版で開いても、版の印は戻さない（戻すと、また新しい版で移し直して失敗する）
    if version < SCHEMA {
        conn.pragma_update(None, "user_version", SCHEMA)?;
    }
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Enriched, Stage};

    fn entity(r: &str) -> Token {
        Token::Entity {
            reference: r.to_owned(),
            spoken: None,
            ruby: None,
        }
    }

    fn text(t: &str) -> Token {
        Token::Text {
            text: t.to_owned(),
            ruby: None,
        }
    }

    fn haze() -> Segment {
        Segment {
            id: "mic-1-1".to_owned(),
            started_at: 5,
            spoken_at: 1,
            stage: Stage::Enriched,
            lang: "ja".to_owned(),
            text: "ヘイズ の ウルト、ヘイズ強い".to_owned(),
            enriched: Some(Enriched {
                source: vec![
                    entity("hero:13"),
                    text("の"),
                    entity("ability:731943444"),
                    text("、"),
                    entity("hero:13"),
                    entity("person:me"),
                    entity("term:soul"),
                    text("強い"),
                ],
                translations: Default::default(),
            }),
            tokens: None,
            draft: None,
            settled_at: Some(2),
        }
    }

    #[test]
    fn counts_speech_and_game_names_but_not_people() {
        let dir = tempfile::tempdir().unwrap();
        let tally = Tally::open(&dir.path().join("stats.db"));
        tally.set_running(true);
        tally.count(&haze());
        let view = tally.view();
        let [session] = view.sessions.as_slice() else {
            panic!("one session: {view:?}");
        };
        assert!(session.live);
        let s = &session.totals;
        assert_eq!(s.utterances, 1);
        assert_eq!(s.characters, 13);
        assert_eq!(s.words, 3);
        assert_eq!(
            s.names,
            BTreeMap::from([
                ("ability:731943444".to_owned(), 1),
                ("hero:13".to_owned(), 2)
            ])
        );
        for period in [&view.week, &view.month, &view.all] {
            assert_eq!(period.totals, *s);
        }
        let [day] = view.days.as_slice() else {
            panic!("one day: {:?}", view.days);
        };
        assert!(day.start <= session.started_at && session.started_at - day.start < 86_400_000);
        assert_eq!((day.utterances, day.characters), (1, 13));
        // 字幕ごとの行も残る（話し始めた時刻と言語）
        let db = tally.db.lock().unwrap();
        let row: (i64, u64, String, u64, u64) = db
            .conn
            .query_row(
                "SELECT session_id, started_at, lang, characters, words FROM utterances",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(row, (session.id, 1, "ja".to_owned(), 13, 3));
    }

    #[test]
    fn forgetting_removes_a_session_with_its_rows_and_keeps_counting() {
        let dir = tempfile::tempdir().unwrap();
        let tally = Tally::open(&dir.path().join("stats.db"));
        tally.set_running(true);
        tally.count(&haze());
        tally.set_running(false);
        tally.set_running(true);
        tally.count(&haze());
        let ids: Vec<i64> = tally.view().sessions.iter().map(|s| s.id).collect();
        let [live, old] = ids.as_slice() else {
            panic!("two sessions: {ids:?}");
        };
        let rows = |tally: &Tally| -> (u64, u64) {
            let db = tally.db.lock().unwrap();
            db.conn
                .query_row(
                    "SELECT (SELECT COUNT(*) FROM session_names), (SELECT COUNT(*) FROM utterances)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap()
        };

        tally.forget(Some(*old)).unwrap();
        let view = tally.view();
        assert_eq!(
            view.sessions.iter().map(|s| s.id).collect::<Vec<_>>(),
            [*live]
        );
        assert_eq!(view.all.totals.utterances, 1);
        assert_eq!(rows(&tally), (2, 1));

        // 出している回を消しても、続きは新しい回として数える
        tally.forget(Some(*live)).unwrap();
        assert!(tally.view().sessions.is_empty());
        tally.count(&haze());
        let view = tally.view();
        let [session] = view.sessions.as_slice() else {
            panic!("one session: {view:?}");
        };
        assert!(session.live && session.id != *live);

        tally.forget(None).unwrap();
        let view = tally.view();
        assert!(view.sessions.is_empty() && view.days.is_empty());
        assert_eq!(view.all.totals.utterances, 0);
        assert_eq!(rows(&tally), (0, 0));
    }

    #[test]
    fn each_start_is_a_new_session_and_empty_ones_are_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let tally = Tally::open(&dir.path().join("stats.db"));
        tally.set_running(true);
        tally.count(&haze());
        tally.set_running(false);
        // 止めたあとに仕上がった字幕は、止めた回に数える
        tally.count(&haze());
        // 何も話さずに止めた回は残さない
        tally.set_running(true);
        tally.set_running(false);
        tally.set_running(true);
        tally.count(&haze());

        let view = tally.view();
        let counts: Vec<_> = view
            .sessions
            .iter()
            .map(|s| (s.totals.utterances, s.live))
            .collect();
        assert_eq!(counts, [(1, true), (2, false)]);
        assert_eq!(view.month.totals.utterances, 3);
        assert_eq!(view.month.totals.names["hero:13"], 6);
        let length: u64 = view
            .sessions
            .iter()
            .map(|s| s.ended_at - s.started_at)
            .sum();
        assert_eq!(view.all.duration_ms, length);
    }

    #[test]
    fn sessions_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        // 初めて起動したときは、データの置き場所がまだない
        let path = dir.path().join("first-run").join("stats.db");
        let tally = Tally::open(&path);
        tally.set_running(true);
        tally.count(&haze());
        drop(tally);
        let view = Tally::open(&path).view();
        assert_eq!(view.sessions.len(), 1);
        assert!(!view.sessions[0].live);
        assert_eq!(view.month.totals.utterances, 1);
    }

    #[test]
    fn time_is_split_into_hours() {
        let dir = tempfile::tempdir().unwrap();
        let tally = Tally::open(&dir.path().join("stats.db"));
        const HOUR: u64 = 3_600_000;
        // 3 時間前の、ある時の 30 分から 90 分（その時に 30 分、次の時に 60 分）
        let start = (now_ms() / HOUR - 3) * HOUR + HOUR / 2;
        tally
            .db
            .lock()
            .unwrap()
            .conn
            .execute(
                "INSERT INTO sessions (started_at, ended_at, utterances) VALUES (?1, ?2, 1)",
                params![start, start + HOUR * 3 / 2],
            )
            .unwrap();
        let view = tally.view();
        let filled: Vec<u64> = view.hours.iter().copied().filter(|&ms| ms > 0).collect();
        assert_eq!(view.hours.len(), 7 * 24);
        assert_eq!(filled.iter().sum::<u64>(), HOUR * 3 / 2);
        assert_eq!(filled.len(), 2);
        assert_eq!(view.all.duration_ms, HOUR * 3 / 2);
        assert_eq!(view.all.active_days, 1);
    }
}
