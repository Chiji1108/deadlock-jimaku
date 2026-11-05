//! 状態ストア + broadcast（DESIGN.md 10 章）。状態はサーバーが持ち、変更は最新の状態をそのまま流す。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{broadcast, watch};

use crate::assets::Loaded;
use crate::settings::{self, Settings, SettingsPatch};
use crate::types::{ItemCardsView, RankState, Segment, ServerMsg, Stage, TranscriptionError};
use crate::usage::Meter;

/// snapshot で送る直近の Segment の数
const KEEP_SEGMENTS: usize = 50;

pub struct Store {
    /// 変わったら `runtime` が処理を張り直す
    settings: watch::Sender<Settings>,
    /// 設定・辞書・キャッシュを置く場所
    data_dir: PathBuf,
    /// 設定ファイルを読めなかった理由。そのときは変更をメモリにだけ当てて保存しない（既定値で上書きしない）。
    /// 保存の読み込み〜書き込みをまとめて排他し、同時の変更が消えないようにする
    unsaved: Mutex<Option<String>>,
    /// 字幕を始められなかった理由（操作パネルに出す）
    transcription_error: Mutex<Option<TranscriptionError>>,
    segments: Mutex<VecDeque<Segment>>,
    glossary: watch::Sender<Option<Loaded>>,
    rank: Mutex<Option<RankState>>,
    /// バッジ → アイコン（webp）。バッジごとに持ち、別のバッジのアイコンを配信しない
    rank_image: Mutex<Option<(u32, Bytes)>>,
    tx: broadcast::Sender<ServerMsg>,
    /// マイクの入力レベル（dBFS）。1 秒に 10 回届くので、字幕などと同じ broadcast には流さない
    /// （購読していないオーバーレイを起こさず、取りこぼしの原因にもしない）
    level: watch::Sender<f32>,
    meter: Arc<Meter>,
}

impl Store {
    /// 設定は `settings::load` の結果。読めなかったら既定値で動き、保存はしない
    pub fn new(settings: std::io::Result<Settings>, data_dir: PathBuf) -> Arc<Self> {
        let (settings, unsaved) = match settings {
            Ok(settings) => (settings, None),
            Err(e) => {
                tracing::error!("cannot read settings: {e}; using defaults without saving");
                (Settings::default(), Some(e.to_string()))
            }
        };
        let meter = Arc::new(Meter::new(data_dir.join("usage.json")));
        Arc::new(Self {
            settings: watch::Sender::new(settings),
            data_dir,
            unsaved: Mutex::new(unsaved),
            transcription_error: Mutex::new(None),
            segments: Mutex::new(VecDeque::new()),
            glossary: watch::Sender::new(None),
            rank: Mutex::new(None),
            rank_image: Mutex::new(None),
            tx: broadcast::channel(256).0,
            level: watch::Sender::new(f32::NEG_INFINITY),
            meter,
        })
    }

    pub fn settings(&self) -> Settings {
        self.settings.borrow().clone()
    }

    pub fn watch_settings(&self) -> watch::Receiver<Settings> {
        self.settings.subscribe()
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 操作パネルからの変更を当てて保存する。読めない値なら何も変えずにエラー
    pub fn update_settings(&self, patch: SettingsPatch) -> Result<Settings, String> {
        let unsaved = self.unsaved.lock().unwrap();
        let current = self.settings();
        let next = current.apply(patch)?;
        if next == current {
            return Ok(next);
        }
        if unsaved.is_none() {
            settings::save(&self.data_dir.join(settings::FILE), &next)
                .map_err(|e| e.to_string())?;
        }
        let display = display(&next);
        let display_changed = display != self::display(&current);
        self.settings.send_if_modified(|current| {
            let changed = *current != next;
            *current = next.clone();
            changed
        });
        if display_changed {
            let _ = self.tx.send(display);
        }
        if next.obs_seen != current.obs_seen
            || next.transcription_enabled != current.transcription_enabled
        {
            let _ = self.tx.send(ServerMsg::SettingsChanged);
        }
        Ok(next)
    }

    /// 設定を保存できない理由（操作パネルに出す）
    pub fn unsaved(&self) -> Option<String> {
        self.unsaved.lock().unwrap().clone()
    }

    pub fn set_transcription_error(&self, error: Option<TranscriptionError>) {
        *self.transcription_error.lock().unwrap() = error.clone();
        let _ = self.tx.send(ServerMsg::TranscriptionStatus { error });
    }

    pub fn transcription_error(&self) -> Option<TranscriptionError> {
        self.transcription_error.lock().unwrap().clone()
    }

    /// 字幕を止めたとき、確定していない（表示時間の起点がない）字幕を消す
    pub fn discard_unsettled(&self) {
        let unsettled: Vec<Segment> = self
            .segments
            .lock()
            .unwrap()
            .iter()
            // まだ何も表示していない Segment は流す必要がない（パイプラインの取り消しと同じ）
            .filter(|s| s.settled_at.is_none() && s.stage != Stage::Discarded && !s.text.is_empty())
            .cloned()
            .collect();
        for mut segment in unsettled {
            segment.stage = Stage::Discarded;
            self.upsert_segment(segment);
        }
    }

    /// 設定は変わらないが、処理を張り直す（API キーを変えたときなど）
    pub fn restart(&self) {
        self.settings.send_modify(|_| {});
    }

    pub fn upsert_segment(&self, segment: Segment) {
        {
            let mut segments = self.segments.lock().unwrap();
            match segments.iter_mut().find(|s| s.id == segment.id) {
                Some(s) => *s = segment.clone(),
                None => {
                    segments.push_back(segment.clone());
                    if segments.len() > KEEP_SEGMENTS {
                        segments.pop_front();
                    }
                }
            }
        }
        // 購読者がいなければ送れないだけなので無視する
        let _ = self.tx.send(ServerMsg::SegmentUpsert { segment });
    }

    /// 接続直後と取りこぼし後に送る、すべてのトピックの現在の状態
    pub fn current_state(&self) -> Vec<ServerMsg> {
        let mut msgs = vec![self.snapshot()];
        if let Some(loaded) = self.glossary() {
            msgs.push(ServerMsg::GlossaryUpdated {
                client_version: loaded.glossary.client_version,
            });
        }
        if let Some(rank) = self.rank.lock().unwrap().clone() {
            msgs.push(ServerMsg::RankUpdate { rank: Some(rank) });
        }
        msgs.push(ServerMsg::TranscriptionStatus {
            error: self.transcription_error(),
        });
        msgs.push(display(&self.settings()));
        msgs.push(ServerMsg::Usage {
            usage: self.meter.watch().borrow().clone(),
        });
        msgs
    }

    /// OpenAI の使用量を数える
    pub fn meter(&self) -> Arc<Meter> {
        self.meter.clone()
    }

    pub fn send_level(&self, db: f32) {
        self.level.send_replace(db);
    }

    /// 購読した後の値だけが届く（あとから接続した画面には、次の値から届く）
    pub fn watch_level(&self) -> watch::Receiver<f32> {
        self.level.subscribe()
    }

    pub fn set_rank(&self, rank: RankState) {
        *self.rank.lock().unwrap() = Some(rank.clone());
        let _ = self.tx.send(ServerMsg::RankUpdate { rank: Some(rank) });
    }

    /// ランクのアカウントを外したとき。オーバーレイから消す
    pub fn clear_rank(&self) {
        if self.rank.lock().unwrap().take().is_some() {
            let _ = self.tx.send(ServerMsg::RankUpdate { rank: None });
        }
    }

    pub fn set_rank_image(&self, badge: u32, image: Bytes) {
        *self.rank_image.lock().unwrap() = Some((badge, image));
    }

    /// そのバッジのアイコン（今持っているものだけ）
    pub fn rank_image(&self, badge: u32) -> Option<Bytes> {
        let image = self.rank_image.lock().unwrap();
        image
            .as_ref()
            .filter(|(b, _)| *b == badge)
            .map(|(_, bytes)| bytes.clone())
    }

    pub fn snapshot(&self) -> ServerMsg {
        ServerMsg::Snapshot {
            segments: self.segments.lock().unwrap().iter().cloned().collect(),
        }
    }

    pub fn set_glossary(&self, loaded: Loaded) {
        let client_version = loaded.glossary.client_version;
        self.glossary.send_replace(Some(loaded));
        let _ = self.tx.send(ServerMsg::GlossaryUpdated { client_version });
    }

    pub fn glossary(&self) -> Option<Loaded> {
        self.glossary.borrow().clone()
    }

    /// その言語を頼んで作った辞書ができるまで待つ（言語を変えた直後の古い辞書は使わない）。
    /// 時間内にできなければ None
    pub async fn wait_glossary(&self, lang: &str, timeout: Duration) -> Option<Loaded> {
        let mut rx = self.glossary.subscribe();
        let ready = rx.wait_for(|g| {
            g.as_ref()
                .is_some_and(|l| l.requested.iter().any(|x| x == lang))
        });
        tokio::time::timeout(timeout, ready)
            .await
            .ok()?
            .ok()?
            .clone()
    }

    /// snapshot より先に呼ぶ。間に起きた更新は重複して届くが、upsert なので問題ない
    pub fn subscribe(&self) -> broadcast::Receiver<ServerMsg> {
        self.tx.subscribe()
    }
}

fn display(settings: &Settings) -> ServerMsg {
    ServerMsg::Display {
        subtitles: settings.subtitle_display.clone(),
        rank_language: settings.rank_language(),
        rank_content: settings.rank_content,
        rank_anchor: settings.rank_anchor,
        item_cards: ItemCardsView {
            hold_seconds: settings.item_cards.hold_seconds,
            language: crate::lang::find(&settings.item_language())
                .map_or("english", |l| l.api)
                .to_owned(),
            anchor: settings.item_cards.anchor,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Stage;

    fn seg(id: &str, text: &str) -> Segment {
        Segment {
            id: id.into(),
            started_at: 0,
            stage: Stage::Partial,
            lang: "ja".into(),
            text: text.into(),
            enriched: None,
            settled_at: None,
        }
    }

    fn snapshot_segments(store: &Store) -> Vec<Segment> {
        match store.snapshot() {
            ServerMsg::Snapshot { segments } => segments,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn upsert_replaces_by_id_and_broadcasts() {
        let store = Store::new(Ok(Settings::default()), PathBuf::new());
        let mut rx = store.subscribe();
        store.upsert_segment(seg("a", "さあ"));
        store.upsert_segment(seg("a", "さあ始まりました"));

        assert_eq!(
            snapshot_segments(&store),
            vec![seg("a", "さあ始まりました")]
        );
        assert!(matches!(rx.try_recv(), Ok(ServerMsg::SegmentUpsert { .. })));
        assert!(matches!(rx.try_recv(), Ok(ServerMsg::SegmentUpsert { .. })));
    }

    #[test]
    fn keeps_only_recent_segments() {
        let store = Store::new(Ok(Settings::default()), PathBuf::new());
        for i in 0..KEEP_SEGMENTS + 5 {
            store.upsert_segment(seg(&i.to_string(), ""));
        }
        let segments = snapshot_segments(&store);
        assert_eq!(segments.len(), KEEP_SEGMENTS);
        assert_eq!(segments[0].id, "5");
    }

    #[test]
    fn first_obs_seen_is_announced_once_and_only_changes_are_saved() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(Ok(Settings::default()), dir.path().to_owned());
        let mut rx = store.subscribe();
        let seen = || crate::settings::SettingsPatch {
            obs_seen: Some(true),
            ..Default::default()
        };
        store.update_settings(seen()).unwrap();
        assert!(matches!(rx.try_recv(), Ok(ServerMsg::SettingsChanged)));
        let file = dir.path().join(settings::FILE);
        std::fs::remove_file(&file).unwrap();
        store.update_settings(seen()).unwrap();
        assert!(rx.try_recv().is_err());
        assert!(!file.exists());
    }
}
