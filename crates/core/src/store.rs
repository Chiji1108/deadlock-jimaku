//! 状態ストア + broadcast（DESIGN.md 10 章）。状態はサーバーが持ち、変更は最新の状態をそのまま流す。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{broadcast, watch};

use crate::assets::Loaded;
use crate::settings::Settings;
use crate::types::{RankState, Segment, ServerMsg};

/// snapshot で送る直近の Segment の数
const KEEP_SEGMENTS: usize = 50;

pub struct Store {
    settings: Settings,
    segments: Mutex<VecDeque<Segment>>,
    glossary: watch::Sender<Option<Loaded>>,
    rank: Mutex<Option<RankState>>,
    /// バッジ → アイコン（webp）。バッジごとに持ち、別のバッジのアイコンを配信しない
    rank_image: Mutex<Option<(u32, Bytes)>>,
    tx: broadcast::Sender<ServerMsg>,
}

impl Store {
    pub fn new(settings: Settings) -> Arc<Self> {
        Arc::new(Self {
            settings,
            segments: Mutex::new(VecDeque::new()),
            glossary: watch::Sender::new(None),
            rank: Mutex::new(None),
            rank_image: Mutex::new(None),
            tx: broadcast::channel(256).0,
        })
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
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
            msgs.push(ServerMsg::RankUpdate { rank });
        }
        msgs
    }

    pub fn set_rank(&self, rank: RankState) {
        *self.rank.lock().unwrap() = Some(rank.clone());
        let _ = self.tx.send(ServerMsg::RankUpdate { rank });
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

    /// 辞書ができるまで待つ。時間内にできなければ None
    pub async fn wait_glossary(&self, timeout: Duration) -> Option<Loaded> {
        let mut rx = self.glossary.subscribe();
        let ready = rx.wait_for(Option::is_some);
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
        let store = Store::new(Settings::default());
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
        let store = Store::new(Settings::default());
        for i in 0..KEEP_SEGMENTS + 5 {
            store.upsert_segment(seg(&i.to_string(), ""));
        }
        let segments = snapshot_segments(&store);
        assert_eq!(segments.len(), KEEP_SEGMENTS);
        assert_eq!(segments[0].id, "5");
    }
}
