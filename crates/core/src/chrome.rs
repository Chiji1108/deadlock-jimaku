//! Chrome のモード（DESIGN.md 9.6）。Chrome の起動と、ページが認識した文を字幕にする経路。
//!
//! 全画面のゲームに隠れても認識が止まらないよう、省電力の働きを切った Chrome を専用のプロファイルで起動する。
//! フラグは Chrome の起動時にしか効かないので、ふだんの Chrome のプロファイルとは分ける。
//!
//! 翻訳モデルのダウンロードにはクリックが要るが、起動した Chrome を開発者ツールの仕組み
//! （CDP。この PC の中からだけ接続できる）で操作して、手を煩わせずに済ませる。
//!
//! 字幕の経路が音声の処理と共有するのは、読みと用語（`Annotator`）、発話の形とオーバーレイへの出力だけ

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use crate::DEFAULT_PORT;
use crate::annotate::Annotator;
use crate::settings::Settings;
use crate::store::{ChromeEvent, Store};
use crate::subtitles;
use crate::types::{
    ChromeCommand, ChromeTask, Enriched, Heard, LangCode, LiveState, LiveStatus, Segment, Stage,
    Translatable, Translated, Translations,
};

const FLAGS: [&str; 8] = [
    "--disable-features=CalculateNativeWinOcclusion,UseEcoQoSForBackgroundProcess",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--no-first-run",
    "--no-default-browser-check",
    // 専用のプロファイルで字幕のページしか開かないので、毎回マイクを聞かれないようにする
    "--auto-accept-camera-and-microphone-capture",
    "--remote-debugging-port=0",
];
/// タブやアドレスバーのない小さなウィンドウにする
const WINDOW_SIZE: &str = "--window-size=460,320";
/// Chrome が CDP の窓口を開くまで、ページが読み込まれるまでの待ち時間
const ATTEMPTS: u32 = 100;
const RETRY: Duration = Duration::from_millis(200);
/// ページの準備が要らないか確かめる間隔
const CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// 落とすと Chrome を閉じる
pub struct Browser {
    child: Child,
    watcher: JoinHandle<()>,
}

impl Browser {
    /// Chrome が見つからなければ `NotFound`
    pub fn launch(store: Arc<Store>) -> io::Result<Self> {
        let profile = store.data_dir().join("chrome-profile");
        let _ = fs::remove_file(profile.join("DevToolsActivePort"));
        let child = command(&arguments(&profile))?
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let watcher = tokio::spawn(async move {
            let closed = attach(&profile).await;
            // 字幕を止める前に閉じられたら、操作パネルで開き直せるようにする
            let settings = store.settings();
            if settings.transcription_enabled && settings.plan().uses_chrome() {
                tracing::warn!("chrome: {closed:#?}");
                store.set_chrome_window(Some(LiveStatus {
                    state: LiveState::Failed,
                    detail: Some("Chrome が閉じられました".into()),
                }));
            }
        });
        Ok(Self { child, watcher })
    }

    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // 閉じたのはアプリなので、閉じられたと知らせない
        self.watcher.abort();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn page_origin() -> String {
    format!("http://127.0.0.1:{DEFAULT_PORT}")
}

fn page_url() -> String {
    format!("{}/chrome", page_origin())
}

fn arguments(profile: &Path) -> Vec<String> {
    FLAGS
        .iter()
        .map(|&flag| flag.to_owned())
        .chain([
            format!("--user-data-dir={}", profile.display()),
            WINDOW_SIZE.to_owned(),
            format!("--app={}", page_url()),
        ])
        .collect()
}

/// ページの準備（クリックが要るモデルのダウンロード）を始めさせる。翻訳先を増やすと
/// またダウンロードが要るので、Chrome が閉じるまで続ける（閉じたことも、これで分かる）
async fn attach(profile: &Path) -> anyhow::Result<std::convert::Infallible> {
    let port = active_port(profile).await?;
    let version: Value = crate::http()
        .get(format!("http://127.0.0.1:{port}/json/version"))
        .send()
        .await?
        .json()
        .await?;
    let url = version["webSocketDebuggerUrl"]
        .as_str()
        .context("no debugger url")?;
    let (socket, _) = tokio_tungstenite::connect_async(url).await?;
    let mut cdp = Cdp { socket, id: 0 };
    // `--auto-accept-camera-and-microphone-capture` はマイクを開かせるだけで、権限として残さない。
    // 権限がないと `enumerateDevices` がマイクの名前も ID も隠すので、選んだマイクを探せない。
    // `grantPermissions` はほかの権限を拒むので使わない。許せなくても、ページは既定のマイクで動いて理由を出す
    if let Err(e) = cdp
        .call(None, "Browser.setPermission", mic_permission())
        .await
    {
        tracing::warn!("chrome: {e:#}");
    }
    let session = cdp.page_session().await?;
    loop {
        cdp.call(
            Some(&session),
            "Runtime.evaluate",
            json!({ "expression": "window.jimakuStart?.()", "userGesture": true }),
        )
        .await?;
        tokio::time::sleep(CHECK_INTERVAL).await;
    }
}

fn mic_permission() -> Value {
    json!({
        "origin": page_origin(),
        "permission": { "name": "microphone" },
        "setting": "granted",
    })
}

/// Chrome が開いた CDP のポートはプロファイルのファイルに書かれる
async fn active_port(profile: &Path) -> anyhow::Result<String> {
    for _ in 0..ATTEMPTS {
        if let Ok(file) = fs::read_to_string(profile.join("DevToolsActivePort"))
            && let Some(port) = file.lines().next().filter(|l| !l.is_empty())
        {
            return Ok(port.to_owned());
        }
        tokio::time::sleep(RETRY).await;
    }
    anyhow::bail!("chrome did not open its debugging port")
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Cdp {
    socket: Socket,
    id: u64,
}

impl Cdp {
    async fn call(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> anyhow::Result<Value> {
        self.id += 1;
        let mut message = json!({ "id": self.id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        self.socket.send(Message::text(message.to_string())).await?;
        while let Some(message) = self.socket.next().await {
            let Message::Text(text) = message? else {
                continue;
            };
            let reply: Value = serde_json::from_str(&text)?;
            if reply["id"] == self.id {
                anyhow::ensure!(reply["error"].is_null(), "{method}: {}", reply["error"]);
                return Ok(reply["result"].clone());
            }
        }
        anyhow::bail!("chrome closed")
    }

    /// 字幕のページのタブにつなぐ
    async fn page_session(&mut self) -> anyhow::Result<String> {
        for _ in 0..ATTEMPTS {
            let targets = self.call(None, "Target.getTargets", json!({})).await?;
            let target = targets["targetInfos"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|t| t["url"] == page_url());
            if let Some(target) = target {
                let session = self
                    .call(
                        None,
                        "Target.attachToTarget",
                        json!({ "targetId": target["targetId"], "flatten": true }),
                    )
                    .await?;
                return session["sessionId"]
                    .as_str()
                    .map(ToOwned::to_owned)
                    .context("no session");
            }
            tokio::time::sleep(RETRY).await;
        }
        anyhow::bail!("the page did not open")
    }
}

#[cfg(target_os = "windows")]
fn command(args: &[String]) -> io::Result<Command> {
    let chrome = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|base| std::path::PathBuf::from(base).join(r"Google\Chrome\Application\chrome.exe"))
        .find(|path| path.exists())
        .ok_or(io::ErrorKind::NotFound)?;
    let mut command = Command::new(chrome);
    command.args(args);
    Ok(command)
}

#[cfg(target_os = "macos")]
fn command(args: &[String]) -> io::Result<Command> {
    let chrome =
        std::path::PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
    if !chrome.exists() {
        return Err(io::ErrorKind::NotFound.into());
    }
    let mut command = Command::new(chrome);
    command.args(args);
    Ok(command)
}

/// どちらの名前で入っているかは配布の形で違う
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn command(args: &[String]) -> io::Result<Command> {
    let on_path = |name: &&str| {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).exists()))
    };
    let chrome = ["google-chrome", "google-chrome-stable"]
        .into_iter()
        .find(on_path)
        .ok_or(io::ErrorKind::NotFound)?;
    let mut command = Command::new(chrome);
    command.args(args);
    Ok(command)
}

/// ページに文を訳してもらう。ページが開いていなければ false
pub(crate) fn ask_translation(
    store: &Store,
    id: String,
    translate: Translatable,
    is_final: bool,
) -> bool {
    store.send_chrome_command(ChromeCommand::Translate(ChromeTask {
        id,
        text: translate.text,
        terms: translate.terms,
        is_final,
    }))
}

/// ページが認識した文を字幕にする。止めるまで続く（ページが閉じられても、開き直せば続きが出る）
pub async fn run(store: Arc<Store>) -> anyhow::Result<()> {
    let settings = store.settings();
    let annotator = Annotator::new(store.clone(), &settings).await;
    let mut events = store.subscribe_chrome();
    let mut subtitles = Subtitles::new(store, &settings, annotator);
    loop {
        match events.recv().await {
            Ok(event) => subtitles.on_event(event),
            // 取りこぼした確定の文や訳を待ち続けない
            Err(_) => subtitles.on_closed(),
        }
    }
}

struct Subtitles {
    store: Arc<Store>,
    lang: LangCode,
    annotator: Annotator,
    /// 翻訳を待っている発話
    live: HashMap<String, Segment>,
    current: Option<String>,
    seq: u64,
}

impl Subtitles {
    fn new(store: Arc<Store>, settings: &Settings, annotator: Annotator) -> Self {
        Self {
            store,
            lang: settings.mic_language.clone(),
            annotator,
            live: HashMap::new(),
            current: None,
            seq: 0,
        }
    }

    fn on_event(&mut self, event: ChromeEvent) {
        match event {
            ChromeEvent::Heard(heard) => self.on_heard(heard),
            ChromeEvent::Translated(translated) => self.on_translated(translated),
            ChromeEvent::Closed => self.on_closed(),
        }
    }

    /// 文の始まりは最初の途中の文で分かる。確定で文が空なら、開いている発話を捨てる
    fn on_heard(&mut self, heard: Heard) {
        let Heard { text, is_final, .. } = heard;
        if self.current.is_none() {
            if text.is_empty() {
                return;
            }
            self.seq += 1;
            let segment = subtitles::partial_segment(self.seq, &self.lang);
            self.current = Some(segment.id.clone());
            self.live.insert(segment.id.clone(), segment);
        }
        let Some(id) = self.current.clone() else {
            return;
        };
        if text.is_empty() {
            self.current = None;
            self.discard(&id);
            return;
        }
        let plan = self.store.settings().plan();
        let phase = if is_final { plan.finals } else { plan.partial };
        let (tokens, translate) = self.annotator.annotate(&text, &phase);
        let Some(seg) = self.live.get_mut(&id) else {
            return;
        };
        seg.text = text;
        seg.tokens = Some(tokens);
        if is_final {
            seg.stage = Stage::Final;
            self.current = None;
        }
        self.store.upsert_segment(seg.clone());
        let asked = phase.translation.is_some()
            && ask_translation(&self.store, id.clone(), translate, is_final);
        if is_final && !asked {
            self.settle(&id, Translations::new());
        }
    }

    fn discard(&mut self, id: &str) {
        if let Some(seg) = self.live.remove(id) {
            subtitles::discard(&self.store, seg);
        }
    }

    fn on_translated(&mut self, translated: Translated) {
        let Translated {
            id,
            is_final,
            translations,
        } = translated;
        if is_final {
            self.settle(&id, translations);
            return;
        }
        if let Some(seg) = self.live.get_mut(&id) {
            let translation_only = self.store.settings().plan().translation_only;
            if subtitles::apply_draft(seg, translations, translation_only) {
                self.store.upsert_segment(seg.clone());
            }
        }
    }

    fn settle(&mut self, id: &str, translations: Translations) {
        let Some(seg) = self.live.remove(id) else {
            return;
        };
        let source = seg.tokens.clone().unwrap_or_default();
        let enriched = Enriched {
            source,
            translations,
        };
        let translation_only = self.store.settings().plan().translation_only;
        self.store
            .upsert_segment(subtitles::settled(seg, enriched, translation_only));
    }

    /// ページが閉じられた。開いている発話を捨て、訳を待っている発話は訳なしで閉じる
    fn on_closed(&mut self) {
        if let Some(id) = self.current.take() {
            self.discard(&id);
        }
        let waiting: Vec<String> = self.live.keys().cloned().collect();
        for id in waiting {
            self.settle(&id, Translations::new());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ServerMsg;

    #[test]
    fn opens_the_page_with_its_own_profile() {
        let args = arguments(Path::new("/data/chrome-profile"));
        assert_eq!(args.last().unwrap(), &format!("--app={}", page_url()));
        assert!(args.contains(&"--user-data-dir=/data/chrome-profile".to_owned()));
    }

    #[test]
    fn allows_only_the_page_to_read_the_microphones() {
        let permission = mic_permission();
        assert_eq!(permission["permission"]["name"], "microphone");
        assert_eq!(
            page_url(),
            format!("{}/chrome", permission["origin"].as_str().unwrap())
        );
    }

    /// 全画面のゲームに隠れても止まらず、確認も出さない形で起動する
    #[test]
    fn starts_chrome_without_throttling_or_prompts() {
        let args = arguments(Path::new("/data/chrome-profile"));
        for flag in [
            "--disable-features=CalculateNativeWinOcclusion,UseEcoQoSForBackgroundProcess",
            "--disable-background-timer-throttling",
            "--disable-backgrounding-occluded-windows",
            "--disable-renderer-backgrounding",
            "--no-first-run",
            "--no-default-browser-check",
            "--auto-accept-camera-and-microphone-capture",
            "--remote-debugging-port=0",
            "--window-size=460,320",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
    }

    async fn subtitles() -> (Subtitles, tokio::sync::broadcast::Receiver<ServerMsg>) {
        let settings = Settings {
            speech_engine: crate::settings::SpeechEngine::Chrome,
            translate_to: vec!["en".into()],
            ..Settings::default()
        };
        let store = Store::new(Ok(settings.clone()), std::path::PathBuf::new());
        let messages = store.subscribe();
        let annotator = Annotator::new(store.clone(), &settings).await;
        (Subtitles::new(store, &settings, annotator), messages)
    }

    fn heard(text: &str, is_final: bool) -> Heard {
        Heard {
            text: text.into(),
            is_final,
            cut: 0,
        }
    }

    fn translated(id: &str, is_final: bool) -> Translated {
        Translated {
            id: id.into(),
            is_final,
            translations: [("en".to_owned(), Vec::new())].into(),
        }
    }

    /// 流れた字幕の段階と文
    fn upserts(messages: &mut tokio::sync::broadcast::Receiver<ServerMsg>) -> Vec<Segment> {
        let mut segments = Vec::new();
        while let Ok(message) = messages.try_recv() {
            if let ServerMsg::SegmentUpsert { segment } = message {
                segments.push(segment);
            }
        }
        segments
    }

    fn last_upsert(
        messages: &mut tokio::sync::broadcast::Receiver<ServerMsg>,
    ) -> Option<(Stage, String)> {
        upserts(messages)
            .pop()
            .map(|segment| (segment.stage, segment.text))
    }

    #[tokio::test]
    async fn a_text_goes_through_the_translator_to_the_overlay() {
        let (mut subs, mut messages) = subtitles().await;
        let mut tasks = subs.store.subscribe_chrome_commands();
        subs.on_heard(heard("ヘイズ", false));
        assert_eq!(
            last_upsert(&mut messages),
            Some((Stage::Partial, "ヘイズ".to_owned()))
        );
        let Ok(ChromeCommand::Translate(task)) = tasks.try_recv() else {
            panic!("no task");
        };
        assert!(!task.is_final);
        let id = task.id;

        subs.on_heard(heard("ヘイズが強い", true));
        assert!(subs.current.is_none());
        assert_eq!(
            last_upsert(&mut messages),
            Some((Stage::Final, "ヘイズが強い".to_owned()))
        );
        assert!(matches!(tasks.try_recv(), Ok(ChromeCommand::Translate(t)) if t.is_final));
        subs.on_translated(translated(&id, true));
        assert_eq!(
            last_upsert(&mut messages),
            Some((Stage::Enriched, "ヘイズが強い".to_owned()))
        );
        assert!(subs.live.is_empty());
    }

    /// 文と読みは同時に変わるので、確定でふりがなやアイコンが消える瞬間がない
    #[tokio::test]
    async fn every_update_has_tokens_for_its_text() {
        let (mut subs, mut messages) = subtitles().await;
        let _tasks = subs.store.subscribe_chrome_commands();
        subs.on_heard(heard("呪いの", false));
        subs.on_heard(heard("呪いの遺物", true));
        let id = subs.live.keys().next().unwrap().clone();
        subs.on_translated(translated(&id, true));
        let segments = upserts(&mut messages);
        assert_eq!(segments.len(), 3);
        for segment in segments {
            let tokens = segment.tokens.expect("tokens");
            let written: String = tokens
                .iter()
                .map(|t| match t {
                    crate::types::Token::Text { text, .. } => text.as_str(),
                    crate::types::Token::Entity { spoken, .. } => spoken.as_deref().unwrap(),
                })
                .collect();
            assert_eq!(written, segment.text);
        }
    }

    #[tokio::test]
    async fn a_late_draft_does_not_settle_the_subtitle() {
        let (mut subs, _messages) = subtitles().await;
        let _tasks = subs.store.subscribe_chrome_commands();
        subs.on_heard(heard("リロード", true));
        let id = subs.live.keys().next().unwrap().clone();
        subs.on_translated(translated(&id, false));
        assert_eq!(subs.live[&id].stage, Stage::Final);
    }

    #[tokio::test]
    async fn without_a_page_the_subtitle_settles_without_a_translation() {
        let (mut subs, mut messages) = subtitles().await;
        subs.on_heard(heard("リロード", true));
        assert_eq!(
            last_upsert(&mut messages),
            Some((Stage::Enriched, "リロード".to_owned()))
        );
    }

    #[tokio::test]
    async fn closing_the_page_clears_what_is_waiting() {
        let (mut subs, mut messages) = subtitles().await;
        let _tasks = subs.store.subscribe_chrome_commands();
        subs.on_heard(heard("リロード", true));
        subs.on_heard(heard("ヘイズ", false));
        subs.on_closed();
        assert!(subs.live.is_empty() && subs.current.is_none());
        assert!(last_upsert(&mut messages).is_some());
    }

    #[tokio::test]
    async fn an_empty_final_text_discards_the_open_subtitle() {
        let (mut subs, mut messages) = subtitles().await;
        subs.on_heard(heard("", true));
        assert!(subs.live.is_empty(), "開いている字幕がない");

        subs.on_heard(heard("ヘイズ", false));
        subs.on_heard(heard("", true));
        assert_eq!(
            last_upsert(&mut messages),
            Some((Stage::Discarded, "ヘイズ".to_owned()))
        );
    }
}
