//! PC の中で動く文字起こし（Haas-ja）。speech.cpp（MIT）を別のプロセスで立て、OpenAI と同じ形の口で頼む（DESIGN.md 9.8）。
//!
//! speech.cpp は初めて使うときに GitHub のリリースから取り、SHA-256 を確かめてからデータフォルダに置く（`download.rs`）。
//! モデル（Apache 2.0、約 0.35 GB）は、speech.cpp が初めての起動で Hugging Face から取る（版は speech.cpp が決めている）。
//! 音声はこの PC から出ない。プロセスは字幕を止めると終わる

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use tokio::process::Command;

use crate::download::{self, Package};
use crate::sidecar::{self, Sidecar};

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

/// 動いている speech.cpp のプロセス。`Speech` を落とすか、アプリを終わるとき（`stop`。タスクは落とされずに
/// 終わるので、ここから止める）に終わらせる
static RUNNING: Sidecar = Sidecar::new("speech.cpp");

/// speech.cpp を終わらせる
pub fn stop() {
    RUNNING.stop();
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

impl Speech {
    /// `dir` に speech.cpp とモデルを置き（なければ取る）、サーバーを立てて、頼めるようになるまで待つ
    pub async fn start(dir: &Path) -> anyhow::Result<Self> {
        let executable = installed(dir).await?;
        let port = sidecar::free_port()?;
        let mut command = Command::new(&executable);
        command
            .args(["serve", MODEL, "--port", &port.to_string()])
            .env("SPEECH_MODEL_DIR", dir.join("models"));
        // ここから先で失敗しても、待っている間に捨てられても、プロセスを残さない
        let speech = Self { port };
        RUNNING.serve(command, port, READY_TIMEOUT).await?;
        tracing::info!("speech.cpp {VERSION} serves {MODEL} on port {port}");
        Ok(speech)
    }

    /// OpenAI の `/v1/audio/transcriptions` と同じ形の口に送る
    pub fn transcriptions(&self) -> reqwest::RequestBuilder {
        let url = format!("http://127.0.0.1:{}/v1/audio/transcriptions", self.port);
        // 長い発話を CPU だけで認識しても足りる長さ
        sidecar::client().post(url).timeout(Duration::from_secs(60))
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
