//! PC の中で動く文字起こし（Haas-ja）。speech.cpp（MIT）を別のプロセスで立て、OpenAI と同じ形の口で頼む（DESIGN.md 9.8）。
//!
//! speech.cpp は初めて使うときに GitHub のリリースから取り、SHA-256 を確かめてからデータフォルダに置く。
//! モデル（Apache 2.0、約 0.35 GB）は、speech.cpp が初めての起動で Hugging Face から取る（版は speech.cpp が決めている）。
//! 音声はこの PC から出ない。プロセスは字幕を止めると終わる

use std::io::Cursor;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, bail, ensure};
use sha2::{Digest, Sha256};
use tokio::process::{Child, Command};

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

/// speech.cpp のダウンロード（Windows は 30 MB）。ふだんの API の待ち時間（60 秒）では足りない回線がある
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// 動いている speech.cpp のサーバー
pub struct Speech {
    port: u16,
    /// 落とすとプロセスも終わる（`kill_on_drop`）
    _child: Child,
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
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW（コンソールの窓を出さない）
        let mut child = command
            .spawn()
            .with_context(|| format!("cannot start {}", executable.display()))?;
        let speech_url = format!("http://127.0.0.1:{port}/health");
        let ready = async {
            loop {
                if let Some(status) = child.try_wait()? {
                    bail!("speech.cpp exited: {status}");
                }
                let health = crate::http().get(&speech_url).send().await;
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
        Ok(Self {
            port,
            _child: child,
        })
    }

    /// OpenAI の `/v1/audio/transcriptions` と同じ形の口
    pub fn transcriptions_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1/audio/transcriptions", self.port)
    }
}

/// speech.cpp の実行ファイル。なければ取って置く
async fn installed(dir: &Path) -> anyhow::Result<PathBuf> {
    let (target, sha256) =
        RELEASE.context("speech.cpp has no build for this computer (Haas-ja cannot run here)")?;
    let folder = dir.join(VERSION);
    let executable = folder.join(EXECUTABLE);
    if tokio::fs::try_exists(&executable).await? {
        return Ok(executable);
    }
    let url = format!(
        "https://github.com/nyosegawa/speech.cpp/releases/download/v{VERSION}/speech-{VERSION}-{target}.zip"
    );
    tracing::info!("downloading {url}");
    let res = crate::http().get(&url).timeout(DOWNLOAD_TIMEOUT);
    let res = res.send().await?.error_for_status()?;
    let archive = res.bytes().await?;
    let digest: String = Sha256::digest(&archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ensure!(digest == sha256, "speech.cpp download has a wrong checksum");
    // 途中で止まっても半端なフォルダを使わないよう、別の名前に広げてから置き換える
    let unpacking = dir.join(format!("{VERSION}.tmp"));
    let (from, to) = (unpacking.clone(), folder);
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let _ = std::fs::remove_dir_all(&from);
        std::fs::create_dir_all(&from)?;
        zip::ZipArchive::new(Cursor::new(archive))?.extract(&from)?;
        std::fs::rename(&from, &to)?;
        Ok(())
    })
    .await??;
    Ok(executable)
}
