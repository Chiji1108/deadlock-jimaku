//! PC の中で動く文字起こし（Haas-ja）。speech.cpp（MIT）を別のプロセスで立て、OpenAI と同じ形の口で頼む（DESIGN.md 9.8）。
//!
//! speech.cpp は初めて使うときに GitHub のリリースから取り、SHA-256 を確かめてからデータフォルダに置く（`download.rs`）。
//! モデル（Apache 2.0、約 0.35 GB）は、speech.cpp が初めての起動で Hugging Face から取る（版は speech.cpp が決めている）。
//! 音声はこの PC から出ない。プロセスは字幕を止めると終わる

use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, bail};
use tokio::process::{Child, Command};

use crate::download::{self, Package};

/// speech.cpp でのモデルの名前（料金の表と設定の `models.batch` にも使う）
pub const HAAS_JA: &str = "haas-ja-v1.0";

/// 重みの種類。Q4_K（0.35 GB）でも、既定の Q8_0（0.66 GB）と文字誤り率が変わらなかった（DESIGN.md 9.7）
const MODEL: &str = "haas-ja-v1.0:q4_k";

const VERSION: &str = "0.10.1";

/// この PC で動く配布物の名前と SHA-256（リリースの `SHA256SUMS`）。Intel の Mac の配布物はない
const RELEASE: Option<(&str, &str)> = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some((
        "macos-arm64-metal",
        "ff24f6f58c9d2dedb01caa88ba043ae845c76eff228e4cbfae5de749174404c1",
    ))
} else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    Some((
        "windows-x64-vulkan",
        "a8fe0e4b4f463c460e7ace9d38c363bec08b254cac5a7f764b7967af35d4fa1a",
    ))
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some((
        "linux-x64-cpu",
        "3396d6b2ec38c2922ff0f24a760f7b45c4ac601788f53f03eb77134393bc768b",
    ))
} else {
    None
};

const EXECUTABLE: &str = if cfg!(windows) {
    "speech.exe"
} else {
    "speech"
};

/// 初めての起動は、モデルのダウンロード（約 0.35 GB）を待つ。遅い回線でも待てる長さにする
const READY_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// 動いている speech.cpp のプロセス。同時に動くのは 1 つ（字幕の処理は 1 つしか動かない）。
/// `Speech` を落とすか、アプリを終わるとき（`stop`。タスクは落とされずに終わるので、ここから止める）に終わらせる
static RUNNING: Mutex<Option<Child>> = Mutex::new(None);

/// speech.cpp を終わらせる
pub fn stop() {
    if let Some(mut child) = RUNNING.lock().unwrap().take() {
        let _ = child.start_kill();
    }
}

/// 動いている speech.cpp のサーバー
pub struct Speech {
    port: u16,
}

impl Drop for Speech {
    fn drop(&mut self) {
        stop();
    }
}

/// この PC の中の口に送る。プロキシの設定があっても通さない
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        let client = reqwest::Client::builder().no_proxy();
        client.build().expect("valid client config")
    })
}

impl Speech {
    /// `dir` に speech.cpp とモデルを置き（なければ取る）、サーバーを立てて、頼めるようになるまで待つ
    pub async fn start(dir: &Path) -> anyhow::Result<Self> {
        let executable = installed(dir).await?;
        // 空いているポートを先に決める（speech.cpp に選ばせると、出力から読み取ることになる）
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
            .local_addr()?
            .port();
        let mut command = Command::new(&executable);
        command
            .args(["serve", MODEL, "--port", &port.to_string()])
            .env("SPEECH_MODEL_DIR", dir.join("models"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // このタスクごと捨てられても残さない（ふだんは `stop` が終わらせる）
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW（コンソールの窓を出さない）
        let child = command
            .spawn()
            .with_context(|| format!("cannot start {}", executable.display()))?;
        stop();
        *RUNNING.lock().unwrap() = Some(child);
        // ここから先で失敗しても、待っている間に捨てられても、プロセスを残さない
        let speech = Self { port };
        let speech_url = format!("http://127.0.0.1:{port}/health");
        let ready = async {
            loop {
                let exited = match &mut *RUNNING.lock().unwrap() {
                    Some(child) => child.try_wait()?,
                    None => bail!("speech.cpp was stopped"),
                };
                if let Some(status) = exited {
                    bail!("speech.cpp exited: {status}");
                }
                let health = client().get(&speech_url).send().await;
                if health.is_ok_and(|res| res.status().is_success()) {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        };
        tokio::time::timeout(READY_TIMEOUT, ready)
            .await
            .context("speech.cpp did not get ready")??;
        tracing::info!("speech.cpp {VERSION} serves {MODEL} on port {port}");
        Ok(speech)
    }

    /// OpenAI の `/v1/audio/transcriptions` と同じ形の口に送る
    pub fn transcriptions(&self) -> reqwest::RequestBuilder {
        let url = format!("http://127.0.0.1:{}/v1/audio/transcriptions", self.port);
        // 長い発話を CPU だけで認識しても足りる長さ
        client().post(url).timeout(Duration::from_secs(60))
    }
}

/// speech.cpp の実行ファイル。なければ取って置く
async fn installed(dir: &Path) -> anyhow::Result<PathBuf> {
    let (target, sha256) =
        RELEASE.context("speech.cpp has no build for this computer (Haas-ja cannot run here)")?;
    let url = format!(
        "https://github.com/nyosegawa/speech.cpp/releases/download/v{VERSION}/speech-{VERSION}-{target}.zip"
    );
    let package = Package {
        url: &url,
        sha256,
        folder: VERSION,
        file: EXECUTABLE,
    };
    download::installed(dir, &package).await
}
