//! 状態ストア + broadcast（DESIGN.md 10 章）。状態はサーバーが持ち、変更は最新の状態をそのまま流す。

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{broadcast, watch};

use crate::assets::Loaded;
use crate::chrome::{self, Browser};
use crate::fonts::Library;
use crate::settings::{
    self, Engines, FinalSource, LiveSource, Settings, SettingsPatch, SubtitleDisplay,
};
use crate::types::{
    ChromeCommand, Decided, Heard, ItemCardsView, LiveState, LiveStatus, RankState, Segment,
    ServerMsg, Stage, TranscriptionError, Translated,
};
use crate::usage::Meter;

/// snapshot で送る直近の Segment の数
const KEEP_SEGMENTS: usize = 50;

/// Chrome のページから字幕の処理へ（確定の文を落とさない）。字幕を止めていれば誰も受け取らない
#[derive(Clone, Debug, PartialEq)]
pub enum ChromeEvent {
    Heard(Heard),
    Translated(Translated),
    Decided(Decided),
    /// ページが閉じられた
    Closed,
}

/// 聞いているページ（DESIGN.md 9.6）
#[derive(Default)]
pub struct ChromeTab {
    /// つなぐたびに増える。今より古いページは閉じる
    pub id: u64,
    status: Option<LiveStatus>,
    /// ウィンドウを開けない、閉じられた理由。ページがつながっている間は出さない
    window: Option<LiveStatus>,
}

impl ChromeTab {
    fn view(&self) -> Option<LiveStatus> {
        self.status.clone().or_else(|| self.window.clone())
    }
}

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
    /// ベータのコードで断られて、Chrome の作り方で続けている理由（DESIGN.md 17.2）。保存はしない
    fallback: Mutex<Option<TranscriptionError>>,
    /// PC の中の文字起こしを準備している
    preparing: AtomicBool,
    segments: Mutex<VecDeque<Segment>>,
    glossary: watch::Sender<Option<Loaded>>,
    /// 新しい版の辞書を作り直すよう頼んだ回数。変わると `runtime` が辞書の処理を張り直す
    glossary_refreshes: AtomicU64,
    rank: Mutex<Option<RankState>>,
    tx: broadcast::Sender<ServerMsg>,
    /// マイクの入力レベル（dBFS）。1 秒に 10 回届くので、字幕などと同じ broadcast には流さない
    /// （購読していないオーバーレイを起こさず、取りこぼしの原因にもしない）
    level: watch::Sender<f32>,
    live_status: Mutex<Option<LiveStatus>>,
    chrome_events: broadcast::Sender<ChromeEvent>,
    chrome_commands: broadcast::Sender<ChromeCommand>,
    chrome_tab: watch::Sender<ChromeTab>,
    /// 字幕を出している間の Chrome。落とすと閉じる。利用者が閉じても、字幕を始め直すか開き直すまで持つ
    browser: Mutex<Option<Browser>>,
    /// ハイブリッドの発話を閉じた回数
    chrome_cuts: AtomicU32,
    meter: Arc<Meter>,
    fonts: Arc<Library>,
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
        let fonts = Arc::new(Library::new(&data_dir));
        Arc::new(Self {
            settings: watch::Sender::new(settings),
            data_dir,
            unsaved: Mutex::new(unsaved),
            transcription_error: Mutex::new(None),
            fallback: Mutex::new(None),
            preparing: AtomicBool::new(false),
            segments: Mutex::new(VecDeque::new()),
            glossary: watch::Sender::new(None),
            glossary_refreshes: AtomicU64::new(0),
            rank: Mutex::new(None),
            tx: broadcast::channel(256).0,
            level: watch::Sender::new(f32::NEG_INFINITY),
            live_status: Mutex::new(None),
            chrome_events: broadcast::channel(256).0,
            chrome_commands: broadcast::channel(256).0,
            chrome_tab: watch::Sender::new(ChromeTab::default()),
            browser: Mutex::new(None),
            chrome_cuts: AtomicU32::new(0),
            meter,
            fonts,
        })
    }

    /// 今の字幕の作り方。ベータのコードで断られている間は、工程だけ Chrome のモードにしたもの
    pub fn settings(&self) -> Settings {
        let mut settings = self.saved();
        if self.fallback().is_some() {
            settings.engines = Engines::CHROME;
        }
        settings
    }

    /// 保存してある設定（利用者が選んだもの）
    pub fn saved(&self) -> Settings {
        self.settings.borrow().clone()
    }

    pub fn fallback(&self) -> Option<TranscriptionError> {
        self.fallback.lock().unwrap().clone()
    }

    /// ベータのコードで断られたら、字幕を止めずに Chrome の作り方で続ける。配信の途中で字幕が消えるのが
    /// いちばん困るため。続けるなら true。途中の字幕を Chrome で作っていないときは続けない（開いていない
    /// Chrome を開いて、選んでいない送り先に音声を送ることになる）
    pub fn fall_back(&self, refused: &TranscriptionError) -> bool {
        let beta = matches!(
            refused,
            TranscriptionError::BetaLimit | TranscriptionError::BetaRefused
        );
        if !beta || self.saved().engines.live != LiveSource::Chrome {
            return false;
        }
        tracing::info!("beta refused ({refused:?}); continuing with Chrome");
        *self.fallback.lock().unwrap() = Some(refused.clone());
        let _ = self.tx.send(ServerMsg::SettingsChanged);
        self.restart();
        true
    }

    pub fn watch_settings(&self) -> watch::Receiver<Settings> {
        self.settings.subscribe()
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn fonts(&self) -> &Arc<Library> {
        &self.fonts
    }

    /// 操作パネルからの変更を当てて保存する。読めない値なら何も変えずにエラー
    pub fn update_settings(&self, patch: SettingsPatch) -> Result<Settings, String> {
        let unsaved = self.unsaved.lock().unwrap();
        let current = self.saved();
        let next = current.apply(patch)?;
        if next == current {
            return Ok(next);
        }
        // 字幕を止めるか、作り方を選び直したら、次は選んだ作り方で始める（まだ断られるなら、また Chrome で続ける）
        if next.engines != current.engines || !next.transcription_enabled {
            *self.fallback.lock().unwrap() = None;
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
        if !self.chrome_needed() {
            self.close_chrome();
        }
        let preparing = self.preparing.load(Ordering::Relaxed);
        let _ = self
            .tx
            .send(ServerMsg::TranscriptionStatus { error, preparing });
    }

    /// 初回のダウンロード（PC の中の文字起こし）を待っている間、操作パネルに出す。
    /// 返したものを捨てると（待っている処理ごと捨てられても）、表示を戻す
    pub fn preparing(self: &Arc<Self>) -> Preparing {
        self.set_preparing(true);
        Preparing(self.clone())
    }

    /// 準備の間は Chrome を閉じ、終わったら開く
    fn set_preparing(self: &Arc<Self>, preparing: bool) {
        self.preparing.store(preparing, Ordering::Relaxed);
        self.sync_chrome();
        let error = self.transcription_error();
        let _ = self
            .tx
            .send(ServerMsg::TranscriptionStatus { error, preparing });
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
            preparing: self.preparing.load(Ordering::Relaxed),
        });
        msgs.push(ServerMsg::LiveStatus {
            status: self.live_status.lock().unwrap().clone(),
        });
        msgs.push(ServerMsg::ChromeStatus {
            status: self.chrome_tab.borrow().view(),
        });
        msgs.push(display(&self.settings()));
        msgs.push(ServerMsg::Usage {
            usage: self.meter.watch().borrow().clone(),
        });
        msgs
    }

    /// 字幕の処理が文を受け取れるか。変わったときだけ知らせる
    pub fn set_live_status(&self, status: Option<LiveStatus>) {
        let mut current = self.live_status.lock().unwrap();
        if *current != status {
            current.clone_from(&status);
            let _ = self.tx.send(ServerMsg::LiveStatus { status });
        }
    }

    /// 変わったときだけ知らせる
    fn change_chrome_tab(&self, change: impl FnOnce(&mut ChromeTab)) {
        self.chrome_tab.send_modify(|tab| {
            let before = tab.view();
            change(tab);
            let status = tab.view();
            if status != before {
                let _ = self.tx.send(ServerMsg::ChromeStatus { status });
            }
        });
    }

    /// ページがつながった。聞くのは 1 つだけなので、今のページに代わる（聞いているページはこれで閉じる）。
    /// 戻り値はこのページの番号と、代わられたと分かるもの
    pub fn connect_chrome(&self) -> (u64, watch::Receiver<ChromeTab>) {
        let mut replaced = false;
        let mut id = 0;
        self.change_chrome_tab(|tab| {
            tab.id += 1;
            id = tab.id;
            replaced = tab.status.is_some();
            // 読み込み直されたので、落ちたという案内は要らない。ほかの案内は開き直すまで残す
            let crashed = |window: &LiveStatus| window.detail.as_deref() == Some(chrome::CRASHED);
            if tab.window.as_ref().is_some_and(crashed) {
                tab.window = None;
            }
        });
        if replaced {
            self.send_chrome_event(ChromeEvent::Closed);
        }
        (id, self.chrome_tab.subscribe())
    }

    /// 古いページの知らせは捨てる
    pub fn set_chrome_status(&self, id: u64, status: LiveStatus) {
        self.change_chrome_tab(|tab| {
            if tab.id == id {
                tab.status = Some(status);
            }
        });
    }

    /// ページが切れた。代わられたあとの古いページなら、今のページの状態を消さない
    pub fn chrome_tab_closed(&self, id: u64) {
        self.change_chrome_tab(|tab| {
            if tab.id == id {
                tab.status = None;
                self.send_chrome_event(ChromeEvent::Closed);
            }
        });
    }

    /// ハイブリッドの発話を閉じた。番号は字幕の処理を作り直しても戻らないので、ページとずれない
    pub fn cut_chrome(&self) {
        let count = self.chrome_cuts.fetch_add(1, Ordering::Relaxed) + 1;
        self.send_chrome_command(ChromeCommand::Cut { count });
    }

    pub fn chrome_cuts(&self) -> u32 {
        self.chrome_cuts.load(Ordering::Relaxed)
    }

    /// 今操作パネルに出している、ページの状態
    pub fn snapshot_chrome_status(&self) -> Option<LiveStatus> {
        self.chrome_tab.borrow().view()
    }

    pub fn send_chrome_event(&self, event: ChromeEvent) {
        let _ = self.chrome_events.send(event);
    }

    pub fn subscribe_chrome(&self) -> broadcast::Receiver<ChromeEvent> {
        self.chrome_events.subscribe()
    }

    /// ページが開いていなければ false
    pub fn send_chrome_command(&self, command: ChromeCommand) -> bool {
        self.chrome_commands.send(command).is_ok()
    }

    pub fn subscribe_chrome_commands(&self) -> broadcast::Receiver<ChromeCommand> {
        self.chrome_commands.subscribe()
    }

    /// Chrome のウィンドウの問題を操作パネルに出す
    pub fn set_chrome_window(&self, problem: Option<LiveStatus>) {
        self.change_chrome_tab(|tab| tab.window = problem);
    }

    /// Chrome を開いておくか。字幕にならない音声を Google に送らない: 鍵の要る工程があるか、確定をアプリが作る
    /// （Chrome だけでは字幕にならない）のに字幕が止まっているとき（鍵がないときも）と、PC の中の文字起こしを
    /// 準備している間は開かない
    fn chrome_needed(&self) -> bool {
        let settings = self.settings();
        let plan = settings.plan();
        let engines = settings.engines;
        let stalled = (engines.paid() || engines.transcription != FinalSource::Chrome)
            && self.transcription_error().is_some();
        let preparing = self.preparing.load(Ordering::Relaxed);
        settings.transcription_enabled && plan.uses_chrome() && !stalled && !preparing
    }

    /// 字幕を始めるなら Chrome を起動し、止めるなら閉じる（DESIGN.md 9.6）。
    /// ロックを持ってから判断するので、字幕が止まるのと重なっても開いたままにならない
    pub fn sync_chrome(self: &Arc<Self>) {
        let mut browser = self.browser.lock().unwrap();
        if !self.chrome_needed() {
            drop(browser);
            self.close_chrome();
            return;
        }
        // 閉じられた Chrome も残るので、設定を変えただけでは起動し直さない
        if browser.is_some() {
            return;
        }
        match Browser::launch(self.clone()) {
            Ok(launched) => {
                *browser = Some(launched);
                drop(browser);
                self.set_chrome_window(None);
            }
            Err(e) => {
                tracing::warn!("open chrome: {e}");
                drop(browser);
                self.set_chrome_window(Some(LiveStatus {
                    state: LiveState::Missing,
                    detail: Some("Google Chrome が必要です".into()),
                }));
            }
        }
    }

    pub fn close_chrome(&self) {
        *self.browser.lock().unwrap() = None;
        self.set_chrome_window(None);
    }

    /// 開き直す。プロセスが残っていても、ページが止まっていることがある
    pub fn reopen_chrome(self: &Arc<Self>) {
        self.close_chrome();
        self.sync_chrome();
    }

    /// Chrome を止めてプロファイルを消し、字幕が動いていれば起動し直す（DESIGN.md 9.6）。
    /// 消し終わるまで `browser` を持ち、消している間に `sync_chrome` が起動しないようにする。
    /// 消せなくても、字幕を止めたままにしない
    pub fn reset_chrome(self: &Arc<Self>) -> io::Result<()> {
        let removed = {
            let mut browser = self.browser.lock().unwrap();
            *browser = None;
            chrome::remove_profile(&self.data_dir)
        };
        self.sync_chrome();
        removed
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

    /// 取ってきたランクのアイコン（バッジごとの webp）を置く場所
    pub fn rank_images(&self) -> PathBuf {
        self.data_dir.join("rank")
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

    /// 辞書を作り直す。できたら `set_glossary` が知らせ、用語の表も差し替わる
    pub fn refresh_glossary(&self) {
        self.glossary_refreshes.fetch_add(1, Ordering::Relaxed);
        self.restart();
    }

    pub fn glossary_refreshes(&self) -> u64 {
        self.glossary_refreshes.load(Ordering::Relaxed)
    }

    pub fn glossary(&self) -> Option<Loaded> {
        self.glossary.borrow().clone()
    }

    pub fn watch_glossary(&self) -> watch::Receiver<Option<Loaded>> {
        self.glossary.subscribe()
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
        // 翻訳先がなければ話した言葉を出す
        subtitles: SubtitleDisplay {
            show_source: !settings.plan().translation_only,
            ..settings.subtitle_display.clone()
        },
        subtitle_style: Box::new(settings.subtitle_style.clone()),
        rank_language: settings.rank_language(),
        rank_content: settings.rank_content,
        rank_anchor: settings.rank_anchor,
        rank_style: Box::new(settings.rank_style.clone()),
        item_cards: ItemCardsView {
            hold_seconds: settings.item_cards.hold_seconds,
            language: crate::lang::find(&settings.item_language())
                .map_or("english", |l| l.api)
                .to_owned(),
            anchor: settings.item_cards.anchor,
        },
    }
}

/// 初回のダウンロードを待っている間の印（`Store::preparing`）
pub struct Preparing(Arc<Store>);

impl Drop for Preparing {
    fn drop(&mut self) {
        self.0.set_preparing(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::types::Stage;

    #[test]
    fn a_refused_beta_code_continues_with_chrome_until_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            engines: Engines::BETA,
            transcription_enabled: true,
            ..Settings::default()
        };
        let store = Store::new(Ok(settings), dir.path().to_owned());
        // ギフトコードの理由や、ほかのエラーでは続けない
        assert!(!store.fall_back(&TranscriptionError::GiftLimit));
        assert_eq!(store.settings().engines, Engines::BETA);
        assert!(store.fall_back(&TranscriptionError::BetaLimit));
        // 字幕は Chrome の作り方で続け、選んだモードは変えない（保存もしない）
        assert_eq!(store.settings().engines, Engines::CHROME);
        assert_eq!(store.saved().engines, Engines::BETA);
        assert_eq!(store.fallback(), Some(TranscriptionError::BetaLimit));
        // 関係のない設定を変えても続ける
        let patch = |patch| store.update_settings(patch).unwrap();
        patch(SettingsPatch {
            deadlock_mode: Some(false),
            ..Default::default()
        });
        assert_eq!(store.settings().engines, Engines::CHROME);
        assert_eq!(store.saved().engines, Engines::BETA);
        // 字幕を止めたら、次は選んだモードで始める
        patch(SettingsPatch {
            transcription_enabled: Some(false),
            ..Default::default()
        });
        assert_eq!(store.fallback(), None);
        assert_eq!(store.settings().engines, Engines::BETA);
    }

    #[test]
    fn a_refused_beta_code_stops_when_chrome_is_not_in_use() {
        let settings = Settings {
            engines: Engines {
                live: LiveSource::QwenAudioStreaming,
                ..Engines::BETA
            },
            ..Settings::default()
        };
        let store = Store::new(Ok(settings), PathBuf::new());
        assert!(!store.fall_back(&TranscriptionError::BetaLimit));
        assert_eq!(store.fallback(), None);
    }

    fn seg(id: &str, text: &str) -> Segment {
        Segment {
            id: id.into(),
            started_at: 0,
            stage: Stage::Partial,
            lang: "ja".into(),
            text: text.into(),
            enriched: None,
            tokens: None,
            draft: None,
            settled_at: None,
        }
    }

    fn snapshot_segments(store: &Store) -> Vec<Segment> {
        match store.snapshot() {
            ServerMsg::Snapshot { segments } => segments,
            other => panic!("unexpected {other:?}"),
        }
    }

    fn chrome_store(engines: Engines) -> (tempfile::TempDir, Arc<Store>) {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings {
            engines,
            transcription_enabled: true,
            ..Settings::default()
        };
        let store = Store::new(Ok(settings), dir.path().to_owned());
        (dir, store)
    }

    #[tokio::test]
    async fn a_stopped_hybrid_closes_chrome_and_does_not_open_it_again() {
        let (_dir, store) = chrome_store(Engines::STANDARD);
        *store.browser.lock().unwrap() = Some(Browser::exited());
        store.set_transcription_error(Some(TranscriptionError::GiftRevoked));
        assert!(store.browser.lock().unwrap().is_none());
        store.sync_chrome();
        assert!(store.browser.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn a_missing_key_keeps_chrome_closed_in_hybrid() {
        let (_dir, store) = chrome_store(Engines::STANDARD);
        store.set_transcription_error(Some(TranscriptionError::MissingKey {
            provider: crate::secrets::Provider::OpenAi,
        }));
        store.sync_chrome();
        assert!(store.browser.lock().unwrap().is_none());
    }

    /// Chrome のモードは OpenAI を使わないので、ウィンドウは残す
    #[tokio::test]
    async fn an_error_leaves_chrome_open_in_chrome_mode() {
        let (_dir, store) = chrome_store(Engines::CHROME);
        *store.browser.lock().unwrap() = Some(Browser::exited());
        store.set_transcription_error(Some(TranscriptionError::Crashed {
            detail: "panic".into(),
        }));
        assert!(store.browser.lock().unwrap().is_some());
    }

    /// PC の中の文字起こしは無料でも、確定をアプリが作る。止まったら Chrome だけでは字幕にならないので閉じる
    #[tokio::test]
    async fn an_error_closes_chrome_when_the_app_makes_the_finals() {
        let local = Engines {
            transcription: FinalSource::HaasJa,
            ..Engines::CHROME
        };
        let (_dir, store) = chrome_store(local);
        *store.browser.lock().unwrap() = Some(Browser::exited());
        store.set_transcription_error(Some(TranscriptionError::Failed {
            detail: "no build".into(),
        }));
        assert!(store.browser.lock().unwrap().is_none());
    }

    /// 閉じられたままなら、起動されて `Browser` が入れ替わることもない
    #[tokio::test]
    async fn a_closed_chrome_stays_closed_when_settings_change() {
        let (_dir, store) = chrome_store(Engines::CHROME);
        let closed = Browser::exited();
        let pid = closed.pid();
        *store.browser.lock().unwrap() = Some(closed);
        store.sync_chrome();
        let browser = store.browser.lock().unwrap();
        assert_eq!(browser.as_ref().map(Browser::pid), Some(pid));
    }

    #[tokio::test]
    async fn sengine_forgets_the_closed_chrome() {
        let (_dir, store) = chrome_store(Engines::CHROME);
        *store.browser.lock().unwrap() = Some(Browser::exited());
        let stop = SettingsPatch {
            transcription_enabled: Some(false),
            ..Default::default()
        };
        store.update_settings(stop).unwrap();
        store.sync_chrome();
        assert!(store.browser.lock().unwrap().is_none());
    }

    /// ページの読み込みで消えるのは落ちたという案内だけで、消えたことは操作パネルに届く
    #[tokio::test]
    async fn a_loaded_page_clears_only_the_crash_notice() {
        let store = Store::new(Ok(Settings::default()), PathBuf::new());
        let window = |detail: &str| {
            Some(LiveStatus {
                state: LiveState::Failed,
                detail: Some(detail.into()),
            })
        };
        store.set_chrome_window(window("Chrome が閉じられました"));
        store.connect_chrome();
        assert_eq!(
            store.snapshot_chrome_status(),
            window("Chrome が閉じられました")
        );

        store.set_chrome_window(window(chrome::CRASHED));
        let mut rx = store.subscribe();
        store.connect_chrome();
        assert_eq!(store.snapshot_chrome_status(), None);
        assert_eq!(
            rx.try_recv().unwrap(),
            ServerMsg::ChromeStatus { status: None }
        );
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

    #[test]
    fn a_chosen_font_reaches_the_overlays() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(Ok(Settings::default()), dir.path().to_owned());
        let mut rx = store.subscribe();
        store
            .update_settings(SettingsPatch {
                subtitle_style: Some(json!({ "languages": { "ja": { "font": "Hiragino Sans" } } })),
                rank_style: Some(json!({ "font": "Georgia" })),
                ..Default::default()
            })
            .unwrap();
        let ServerMsg::Display {
            subtitle_style,
            rank_style,
            ..
        } = rx.try_recv().unwrap()
        else {
            panic!("expected the display message");
        };
        assert_eq!(
            subtitle_style.languages["ja"].font.as_deref(),
            Some("Hiragino Sans")
        );
        assert_eq!(rank_style.font.font.as_deref(), Some("Georgia"));
    }
}
