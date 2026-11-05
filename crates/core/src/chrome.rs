//! 無料モードとハイブリッドモードの Chrome を起動する（DESIGN.md 9.6）。
//!
//! 全画面のゲームに隠れても認識が止まらないよう、省電力の働きを切った Chrome を専用のプロファイルで起動する。
//! フラグは Chrome の起動時にしか効かないので、ふだんの Chrome のプロファイルとは分ける。
//!
//! 翻訳モデルのダウンロードにはクリックが要るが、起動した Chrome を開発者ツールの仕組み
//! （CDP。この PC の中からだけ接続できる）で操作して、手を煩わせずに済ませる

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use crate::DEFAULT_PORT;
use crate::store::Store;
use crate::types::{LiveState, LiveStatus};

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
            if settings.transcription_enabled && settings.speech_engine.uses_chrome() {
                tracing::warn!("chrome: {closed:#?}");
                store.set_live_status(Some(LiveStatus {
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
        .map(|base| PathBuf::from(base).join(r"Google\Chrome\Application\chrome.exe"))
        .find(|path| path.exists())
        .ok_or(io::ErrorKind::NotFound)?;
    let mut command = Command::new(chrome);
    command.args(args);
    Ok(command)
}

#[cfg(target_os = "macos")]
fn command(args: &[String]) -> io::Result<Command> {
    let chrome = PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
    if !chrome.exists() {
        return Err(io::ErrorKind::NotFound.into());
    }
    let mut command = Command::new(chrome);
    command.args(args);
    Ok(command)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn command(args: &[String]) -> io::Result<Command> {
    let mut command = Command::new("google-chrome");
    command.args(args);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_the_page_with_its_own_profile() {
        let args = arguments(Path::new("/data/chrome-profile"));
        assert_eq!(args.last().unwrap(), &format!("--app={}", page_url()));
        assert!(args.contains(&"--user-data-dir=/data/chrome-profile".to_owned()));
    }
}
