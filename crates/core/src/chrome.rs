//! Chrome の音声認識と翻訳（DESIGN.md 9.6）。Chrome の起動と、ページが認識した文を字幕にする経路。
//!
//! 全画面のゲームに隠れても認識が止まらないよう、省電力の働きを切った Chrome を専用のプロファイルで起動する。
//! フラグは Chrome の起動時にしか効かないので、ふだんの Chrome のプロファイルとは分ける。
//!
//! 翻訳モデルのダウンロードにはクリックが要るが、起動した Chrome を開発者ツールの仕組み
//! （CDP。この PC の中からだけ接続できる）で操作して、手を煩わせずに済ませる。
//!
//! 字幕の経路が音声の処理と共有するのは、読みと用語（`Annotator`）、確定の仕上げ（`Finisher`）、
//! 発話の形とオーバーレイへの出力だけ

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tokio_tungstenite::tungstenite::Message;

use crate::DEFAULT_PORT;
use crate::annotate::Annotator;
use crate::finish::{self, Finished, Finisher};
use crate::openai::Access;
use crate::settings::Settings;
use crate::store::{ChromeEvent, Store};
use crate::subtitles;
use crate::types::{
    ChromeCommand, ChromeTask, Heard, LangCode, LiveState, LiveStatus, Segment, Stage,
    TranscriptionError, Translatable, Translated, Translations,
};

const FLAGS: [&str; 7] = [
    "--disable-features=CalculateNativeWinOcclusion,UseEcoQoSForBackgroundProcess",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--no-first-run",
    "--no-default-browser-check",
    "--remote-debugging-port=0",
];
/// タブやアドレスバーのない小さなウィンドウにする
const WINDOW_SIZE: &str = "--window-size=460,320";
/// Chrome が CDP の窓口を開くまで、ページが読み込まれるまでの待ち時間
const ATTEMPTS: u32 = 100;
const RETRY: Duration = Duration::from_millis(200);
/// 子プロセスは本体より遅れて終わる。Windows では、それまでプロファイルのファイルを消せない
const REMOVE_ATTEMPTS: u32 = 25;
/// ページの準備が要らないか確かめる間隔
const CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// 確定の文の訳を待つ長さ。ページが固まっても、字幕を仕上げずに残さない
const FINAL_TRANSLATION_TIMEOUT: Duration = Duration::from_secs(10);
/// ページが落ちたときに操作パネルに出す案内
pub(crate) const CRASHED: &str = "Chrome のページが落ちました。「Chrome を開き直す」で戻せます";

/// 落とすと Chrome を閉じる
pub struct Browser {
    child: Child,
    watcher: JoinHandle<()>,
}

impl Browser {
    /// Chrome が見つからなければ `NotFound`
    pub fn launch(store: Arc<Store>) -> io::Result<Self> {
        let profile = profile_dir(store.data_dir());
        let _ = fs::remove_file(profile.join("DevToolsActivePort"));
        let child = command(&arguments(&profile))?
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let watcher = tokio::spawn(async move {
            let (detail, error) = match connect(&profile).await {
                Ok((cdp, page)) => (
                    "Chrome が閉じられました",
                    drive(cdp, &page, &store).await.unwrap_err(),
                ),
                Err(e) => ("Chrome に接続できません", e),
            };
            tracing::warn!("chrome: {error:#}");
            report_failure(&store, detail);
        });
        Ok(Self { child, watcher })
    }
}

#[cfg(test)]
impl Browser {
    /// もう終わったプロセスの Chrome
    pub fn exited() -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        child.wait().unwrap();
        let watcher = tokio::spawn(async {});
        Self { child, watcher }
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
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

/// 字幕を止める前なら、操作パネルで開き直せるようにする
fn report_failure(store: &Store, detail: &str) {
    let settings = store.settings();
    if settings.transcription_enabled && settings.plan().uses_chrome() {
        store.set_chrome_window(Some(LiveStatus {
            state: LiveState::Failed,
            detail: Some(detail.into()),
        }));
    }
}

fn profile_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("chrome-profile")
}

/// 専用のプロファイルを消す。なければ何もしない。Chrome を止めてから呼ぶ
pub(crate) fn remove_profile(data_dir: &Path) -> io::Result<()> {
    let profile = profile_dir(data_dir);
    let remove = || match fs::remove_dir_all(&profile) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    };
    let mut removed = remove();
    for _ in 1..REMOVE_ATTEMPTS {
        if removed.is_ok() {
            break;
        }
        std::thread::sleep(RETRY);
        removed = remove();
    }
    removed
}

fn page_url() -> String {
    format!("http://127.0.0.1:{DEFAULT_PORT}/chrome")
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

/// ページのタブにつなぐ
async fn connect(profile: &Path) -> anyhow::Result<(Cdp, Page)> {
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
    // つなぐ前に頼むので、つないだあとにタブが閉じられれば必ず知らせが届く
    cdp.call(
        None,
        "Target.setDiscoverTargets",
        json!({ "discover": true }),
    )
    .await?;
    let page = cdp.page().await?;
    Ok((cdp, page))
}

/// ページの準備（クリックが要るモデルのダウンロード）を始めさせる。翻訳先を増やすと
/// またダウンロードが要るので、Chrome が閉じるまで続ける。応答では閉じたと判断しない。
/// ページが落ちると応答は来ず、読み込み直すと `Target crashed` が返る（DESIGN.md 9.6）。
/// 落ちても読み込み直せば同じセッションで動き続けるので、知らせるだけで見張りは続ける
async fn drive(
    mut cdp: Cdp,
    page: &Page,
    store: &Store,
) -> anyhow::Result<std::convert::Infallible> {
    let mut check = tokio::time::interval(CHECK_INTERVAL);
    check.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = check.tick() => {
                let start = json!({ "expression": "window.jimakuStart?.()", "userGesture": true });
                cdp.send(Some(&page.session), "Runtime.evaluate", start).await?;
            }
            message = cdp.socket.next() => {
                let Message::Text(text) = message.context("chrome closed")?? else {
                    continue;
                };
                match page_event(&text, &page.target).as_deref() {
                    Some("Target.targetDestroyed") => anyhow::bail!("the page was closed"),
                    Some("Target.targetCrashed") => {
                        tracing::warn!("chrome: the page crashed");
                        report_failure(store, CRASHED);
                    }
                    _ => {}
                }
            }
        }
    }
}

/// このページのタブについての CDP の知らせの名前
fn page_event(message: &str, target: &str) -> Option<String> {
    let event: Value = serde_json::from_str(message).ok()?;
    let method = event["method"].as_str();
    let method = method.filter(|_| event["params"]["targetId"] == target)?;
    Some(method.to_owned())
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

/// 字幕のページのタブ
struct Page {
    target: String,
    session: String,
}

impl Cdp {
    async fn send(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> anyhow::Result<()> {
        self.id += 1;
        let mut message = json!({ "id": self.id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        self.socket.send(Message::text(message.to_string())).await?;
        Ok(())
    }

    async fn call(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> anyhow::Result<Value> {
        self.send(session, method, params).await?;
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
    async fn page(&mut self) -> anyhow::Result<Page> {
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
                return Ok(Page {
                    target: target["targetId"].as_str().context("no target")?.to_owned(),
                    session: session["sessionId"]
                        .as_str()
                        .context("no session")?
                        .to_owned(),
                });
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

/// ページに確定の文を訳してもらう。頼んだときから聞くので、訳がすぐ届いても取りこぼさない。
/// ページが開いていなければ None
pub(crate) fn ask_final_translation(
    store: &Store,
    id: String,
    translate: Translatable,
) -> Option<broadcast::Receiver<ChromeEvent>> {
    let events = store.subscribe_chrome();
    ask_translation(store, id, translate, true).then_some(events)
}

/// 頼んだ確定の文の訳を待つ。ページが閉じられるか、答えが来なければ訳なし
pub(crate) async fn final_translation(
    mut events: broadcast::Receiver<ChromeEvent>,
    id: &str,
) -> Translations {
    let wait = async {
        loop {
            match events.recv().await {
                Ok(ChromeEvent::Translated(Translated {
                    id: done,
                    is_final: true,
                    translations,
                })) if done == id => return translations,
                // 取りこぼした訳を待ち続けない
                Ok(ChromeEvent::Closed) | Err(_) => return Translations::new(),
                Ok(_) => {}
            }
        }
    };
    tokio::time::timeout(FINAL_TRANSLATION_TIMEOUT, wait)
        .await
        .unwrap_or_else(|_| {
            tracing::warn!("no final translation from the page");
            Translations::new()
        })
}

/// ページが認識した文を字幕にする。止めるまで続く（ページが閉じられても、開き直せば続きが出る）。
/// `access` は確定の仕上げに LLM を使う工程があるときだけ
pub async fn run(store: Arc<Store>, access: Option<Access>) -> anyhow::Result<()> {
    let settings = store.settings();
    let annotator = Annotator::new(store.clone(), &settings).await;
    let finisher = Arc::new(Finisher::new(store.clone(), &settings, access));
    let mut events = store.subscribe_chrome();
    let (done_tx, mut done_rx) = tokio::sync::mpsc::channel(64);
    let mut subtitles = Subtitles::new(store, &settings, annotator, finisher, done_tx);
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => subtitles.on_event(event),
                Err(_) => subtitles.on_closed(),
            },
            Some(finished) = done_rx.recv() => {
                // 断られた鍵へは送らない。待っていた結果は捨てる
                if let Some(refused) = subtitles.on_finished(finished) {
                    subtitles.store.discard_unsettled();
                    return Err(refused.into());
                }
            }
        }
    }
}

struct Subtitles {
    store: Arc<Store>,
    lang: LangCode,
    annotator: Annotator,
    finisher: Arc<Finisher>,
    done_tx: tokio::sync::mpsc::Sender<Finished>,
    /// 確定の仕上げを待っている発話
    live: HashMap<String, Segment>,
    current: Option<String>,
    seq: u64,
    /// 直前の確定字幕（LLM の文脈）
    context: VecDeque<String>,
}

impl Subtitles {
    fn new(
        store: Arc<Store>,
        settings: &Settings,
        annotator: Annotator,
        finisher: Arc<Finisher>,
        done_tx: tokio::sync::mpsc::Sender<Finished>,
    ) -> Self {
        Self {
            store,
            lang: settings.mic_language.clone(),
            annotator,
            finisher,
            done_tx,
            live: HashMap::new(),
            current: None,
            seq: 0,
            context: VecDeque::new(),
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
        seg.text.clone_from(&text);
        seg.tokens = Some(tokens.clone());
        if !is_final {
            self.store.upsert_segment(seg.clone());
            if phase.translation.is_some() {
                ask_translation(&self.store, id, translate, false);
            }
            return;
        }
        seg.stage = Stage::Final;
        self.current = None;
        self.store.upsert_segment(seg.clone());
        let context = subtitles::push_context(&mut self.context, &text);
        let heard = finish::Heard {
            id,
            text,
            context,
            tokens,
        };
        self.finisher
            .spawn(heard, translate, phase, self.done_tx.clone());
    }

    fn discard(&mut self, id: &str) {
        if let Some(seg) = self.live.remove(id) {
            subtitles::discard(&self.store, seg);
        }
    }

    /// 途中の文の訳。確定の文の訳は `Finisher` が待つ
    fn on_translated(&mut self, translated: Translated) {
        let Translated {
            id,
            is_final: false,
            translations,
        } = translated
        else {
            return;
        };
        if let Some(seg) = self.live.get_mut(&id) {
            let translation_only = self.store.settings().plan().translation_only;
            if subtitles::apply_draft(seg, translations, translation_only) {
                self.store.upsert_segment(seg.clone());
            }
        }
    }

    /// 断られたら、その理由を返す
    fn on_finished(&mut self, finished: Finished) -> Option<TranscriptionError> {
        let translation_only = self.store.settings().plan().translation_only;
        match finished {
            Finished::Draft { id, translations } => {
                if let Some(seg) = self.live.get_mut(&id) {
                    subtitles::apply_llm_draft(seg, translations, translation_only);
                    self.store.upsert_segment(seg.clone());
                }
            }
            Finished::Done { id, enriched } => {
                if let Some(seg) = self.live.remove(&id) {
                    self.store
                        .upsert_segment(subtitles::settled(seg, enriched, translation_only));
                }
            }
            Finished::Refused(refused) => return Some(refused),
        }
        None
    }

    /// ページが閉じられた。開いている発話を捨てる。訳を待っている発話は訳なしで仕上がる（`final_translation`）
    fn on_closed(&mut self) {
        if let Some(id) = self.current.take() {
            self.discard(&id);
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

    /// 全画面のゲームに隠れても止まらない形で起動する。マイクの許可は利用者に聞く
    #[test]
    fn starts_chrome_without_throttling() {
        let args = arguments(Path::new("/data/chrome-profile"));
        for flag in [
            "--disable-features=CalculateNativeWinOcclusion,UseEcoQoSForBackgroundProcess",
            "--disable-background-timer-throttling",
            "--disable-backgrounding-occluded-windows",
            "--disable-renderer-backgrounding",
            "--no-first-run",
            "--no-default-browser-check",
            "--remote-debugging-port=0",
            "--window-size=460,320",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        assert!(!args.iter().any(|a| a.contains("auto-accept")));
    }

    /// 偽の Chrome を `drive` で見張る
    async fn watch_fake_chrome() -> (JoinHandle<anyhow::Error>, FakeChrome) {
        watch_fake_chrome_for(running_store()).await
    }

    fn running_store() -> Arc<Store> {
        let settings = Settings {
            engines: crate::settings::Engines::CHROME,
            transcription_enabled: true,
            ..Settings::default()
        };
        Store::new(Ok(settings), std::path::PathBuf::new())
    }

    async fn watch_fake_chrome_for(store: Arc<Store>) -> (JoinHandle<anyhow::Error>, FakeChrome) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let client = tokio::spawn(async move {
            let (socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
            let page = Page {
                target: "page".into(),
                session: "session".into(),
            };
            drive(Cdp { socket, id: 0 }, &page, &store)
                .await
                .unwrap_err()
        });
        let (socket, _) = listener.accept().await.unwrap();
        let chrome = tokio_tungstenite::accept_async(socket).await.unwrap();
        (client, chrome)
    }

    type FakeChrome = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

    async fn next_check(chrome: &mut FakeChrome) -> Value {
        let Some(Ok(Message::Text(text))) = chrome.next().await else {
            panic!("no command");
        };
        let command: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(command["method"], "Runtime.evaluate");
        assert_eq!(command["sessionId"], "session");
        command["id"].clone()
    }

    fn target_event(method: &str, target: &str) -> Message {
        let event = json!({ "method": method, "params": { "targetId": target } });
        Message::text(event.to_string())
    }

    fn target_destroyed(target: &str) -> Message {
        target_event("Target.targetDestroyed", target)
    }

    /// Chrome 156 の測定: ページが落ちると応答は来ず、読み込み直すと `Target crashed` が返る
    #[tokio::test]
    async fn a_failed_or_silent_page_is_not_a_closed_window() {
        let (client, mut chrome) = watch_fake_chrome().await;
        let id = next_check(&mut chrome).await;
        let crashed = json!({ "id": id, "error": { "code": -32000, "message": "Target crashed" } });
        chrome
            .send(Message::text(crashed.to_string()))
            .await
            .unwrap();
        next_check(&mut chrome).await;
        // 応答しなくても、次も確かめる
        next_check(&mut chrome).await;
        chrome.send(target_destroyed("another")).await.unwrap();
        next_check(&mut chrome).await;
        assert!(!client.is_finished());

        chrome.send(target_destroyed("page")).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), client)
            .await
            .expect("the closed page ends the watch")
            .unwrap();
    }

    /// ウィンドウの再読み込みでも戻るので、見張りは続ける（DESIGN.md 9.6）
    #[tokio::test]
    async fn a_crashed_page_is_shown_on_the_panel() {
        let store = running_store();
        let (client, mut chrome) = watch_fake_chrome_for(store.clone()).await;
        next_check(&mut chrome).await;
        let crashed = |target| target_event("Target.targetCrashed", target);

        chrome.send(crashed("another")).await.unwrap();
        next_check(&mut chrome).await;
        assert_eq!(store.snapshot_chrome_status(), None);

        chrome.send(crashed("page")).await.unwrap();
        next_check(&mut chrome).await;
        let status = store.snapshot_chrome_status().unwrap();
        assert_eq!(status.state, LiveState::Failed);
        assert!(status.detail.unwrap().contains("Chrome を開き直す"));
        assert!(!client.is_finished());

        let (id, _) = store.connect_chrome();
        store.set_chrome_status(
            id,
            LiveStatus {
                state: LiveState::Listening,
                detail: None,
            },
        );
        store.chrome_tab_closed(id);
        assert_eq!(
            store.snapshot_chrome_status(),
            None,
            "読み込み直したあとに古い案内を出さない"
        );
    }

    #[tokio::test]
    async fn the_end_of_chrome_ends_the_watch() {
        let (client, mut chrome) = watch_fake_chrome().await;
        next_check(&mut chrome).await;
        drop(chrome);
        tokio::time::timeout(Duration::from_secs(5), client)
            .await
            .expect("the lost connection ends the watch")
            .unwrap();
    }

    type Messages = tokio::sync::broadcast::Receiver<ServerMsg>;
    type Done = tokio::sync::mpsc::Receiver<Finished>;

    async fn subtitles() -> (Subtitles, Messages, Done) {
        let settings = Settings {
            engines: crate::settings::Engines::CHROME,
            translate_to: vec!["en".into()],
            ..Settings::default()
        };
        let store = Store::new(Ok(settings.clone()), std::path::PathBuf::new());
        let messages = store.subscribe();
        let annotator = Annotator::new(store.clone(), &settings).await;
        let finisher = Arc::new(Finisher::new(store.clone(), &settings, None));
        let (done_tx, done) = tokio::sync::mpsc::channel(8);
        let subs = Subtitles::new(store, &settings, annotator, finisher, done_tx);
        (subs, messages, done)
    }

    /// 確定の仕上げが終わるのを待って、字幕の処理に渡す
    async fn finish_next(subs: &mut Subtitles, done: &mut Done) {
        let finished = tokio::time::timeout(Duration::from_secs(1), done.recv())
            .await
            .expect("finished")
            .unwrap();
        subs.on_finished(finished);
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
        let (mut subs, mut messages, mut done) = subtitles().await;
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
        subs.store
            .send_chrome_event(ChromeEvent::Translated(translated(&id, true)));
        finish_next(&mut subs, &mut done).await;
        assert_eq!(
            last_upsert(&mut messages),
            Some((Stage::Enriched, "ヘイズが強い".to_owned()))
        );
        assert!(subs.live.is_empty());
    }

    /// 文と読みは同時に変わるので、確定でふりがなやアイコンが消える瞬間がない
    #[tokio::test]
    async fn every_update_has_tokens_for_its_text() {
        let (mut subs, mut messages, mut done) = subtitles().await;
        let _tasks = subs.store.subscribe_chrome_commands();
        subs.on_heard(heard("呪いの", false));
        subs.on_heard(heard("呪いの遺物", true));
        let id = subs.live.keys().next().unwrap().clone();
        subs.store
            .send_chrome_event(ChromeEvent::Translated(translated(&id, true)));
        finish_next(&mut subs, &mut done).await;
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
        let (mut subs, _messages, _done) = subtitles().await;
        let _tasks = subs.store.subscribe_chrome_commands();
        subs.on_heard(heard("リロード", true));
        let id = subs.live.keys().next().unwrap().clone();
        subs.on_translated(translated(&id, false));
        assert_eq!(subs.live[&id].stage, Stage::Final);
    }

    #[tokio::test]
    async fn without_a_page_the_subtitle_settles_without_a_translation() {
        let (mut subs, mut messages, mut done) = subtitles().await;
        subs.on_heard(heard("リロード", true));
        finish_next(&mut subs, &mut done).await;
        assert_eq!(
            last_upsert(&mut messages),
            Some((Stage::Enriched, "リロード".to_owned()))
        );
    }

    #[tokio::test]
    async fn closing_the_page_clears_what_is_waiting() {
        let (mut subs, mut messages, mut done) = subtitles().await;
        let _tasks = subs.store.subscribe_chrome_commands();
        subs.on_heard(heard("リロード", true));
        subs.on_heard(heard("ヘイズ", false));
        subs.store.send_chrome_event(ChromeEvent::Closed);
        subs.on_closed();
        finish_next(&mut subs, &mut done).await;
        assert!(subs.live.is_empty() && subs.current.is_none());
        assert!(last_upsert(&mut messages).is_some());
    }

    #[tokio::test]
    async fn an_empty_final_text_discards_the_open_subtitle() {
        let (mut subs, mut messages, _done) = subtitles().await;
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
