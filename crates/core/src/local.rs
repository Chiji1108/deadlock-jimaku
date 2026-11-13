//! PC の中で動く文字起こし（Haas-ja）。speech.cpp（MIT）を別のプロセスで立て、OpenAI と同じ形の口で頼む（DESIGN.md 9.8）。
//!
//! speech.cpp は初めて使うときに GitHub のリリースから取り、SHA-256 を確かめてからデータフォルダに置く（`download.rs`）。
//! モデル（Apache 2.0、約 0.35 GB）は、speech.cpp が初めての起動で Hugging Face から取る（版は speech.cpp が決めている）。
//! 音声はこの PC から出ない。プロセスは字幕を止めると終わる

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use serde::Serialize;
use tokio::process::Command;
use ts_rs::TS;

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
    /// `dir` に speech.cpp とモデルを置き（なければ取る）、サーバーを立てて、頼めるようになるまで待つ。
    /// `gpu` は使う GPU（None なら speech.cpp に任せる）
    pub async fn start(dir: &Path, gpu: Option<&Gpu>) -> anyhow::Result<Self> {
        let executable = installed(dir).await?;
        let port = sidecar::free_port()?;
        let mut command = Command::new(&executable);
        command
            .args(["serve", MODEL, "--port", &port.to_string()])
            .env("SPEECH_MODEL_DIR", dir.join("models"));
        if let Some(gpu) = gpu {
            command.args(["--device", &gpu.id]);
        }
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

/// この PC の GPU（speech.cpp の `devices` の 1 行）。PC の中の AI（speech.cpp、whisper.cpp、llama.cpp）は、
/// どれも同じ部品で GPU を数えるので、名前（`Vulkan1` など）と並び順が同じ（Windows の実機で確かめた。DESIGN.md 9.11）
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Gpu {
    /// 実行ファイルに渡す名前（`Vulkan0`、`MTL0` など）
    pub id: String,
    /// 画面に出す名前（`NVIDIA GeForce RTX 3070` など）。設定には、これを保存する
    pub name: String,
    /// CPU に内蔵の GPU。PC のメモリを分け合うので、メモリは多く見えるが遅い
    pub integrated: bool,
    /// メモリ（GB）
    pub gb: f64,
}

impl Gpu {
    /// whisper.cpp に渡す番号（Vulkan の GPU だけ）
    pub fn vulkan_index(&self) -> Option<u32> {
        self.id.strip_prefix("Vulkan")?.parse().ok()
    }
}

/// 操作パネルに出す、GPU の一覧
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct Gpus {
    pub gpus: Vec<Gpu>,
    /// 自動なら使う GPU の名前（GPU が 1 つ以下なら、選ぶものがないので None）
    #[ts(optional)]
    pub auto: Option<String>,
}

impl Gpus {
    pub fn new(gpus: Vec<Gpu>) -> Self {
        let auto = pick(&gpus, None).map(|gpu| gpu.name.clone());
        Self { gpus, auto }
    }
}

/// `speech devices` の出力から、GPU を読む（CPU の行は除く）。1 行は `名前  種類  説明, 全体 GB, 空き GB free`
fn parse_gpus(listed: &str) -> Vec<Gpu> {
    let gpu = |line: &str| {
        let (id, rest) = line.trim().split_once(char::is_whitespace)?;
        let (kind, rest) = rest.trim_start().split_once(char::is_whitespace)?;
        let integrated = match kind {
            "gpu" => false,
            "igpu" => true,
            _ => return None,
        };
        // 説明の終わりの 2 つが、全体と空きのメモリ
        let mut parts = rest.rsplitn(3, ", ");
        let (_free, total, name) = (parts.next()?, parts.next()?, parts.next()?);
        Some(Gpu {
            id: id.to_owned(),
            name: name.trim().to_owned(),
            integrated,
            gb: total.trim().strip_suffix(" GB")?.parse().ok()?,
        })
    };
    listed.lines().filter_map(gpu).collect()
}

/// PC の中の AI に使わせる GPU。`wanted` は設定で選んだ名前。選んでいないか、その GPU が見つからなければ、
/// 内蔵でない GPU を優先し、同じ種類ならメモリの多いもの。GPU が 1 つ以下なら None（実行ファイルに任せる）
pub fn pick<'a>(gpus: &'a [Gpu], wanted: Option<&str>) -> Option<&'a Gpu> {
    if gpus.len() < 2 {
        return None;
    }
    let chosen = wanted.and_then(|name| gpus.iter().find(|gpu| gpu.name == name));
    chosen.or_else(|| {
        gpus.iter().max_by(|a, b| {
            let rank = |gpu: &Gpu| (!gpu.integrated, gpu.gb);
            rank(a)
                .partial_cmp(&rank(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    })
}

/// この PC の GPU。speech.cpp に数えさせる（なければ取る。Mac は 6 MB、Windows は 30 MB）
pub async fn gpus(dir: &Path) -> anyhow::Result<Vec<Gpu>> {
    let executable = installed(dir).await?;
    list_gpus(&executable).await
}

/// この PC の GPU。speech.cpp をまだ取っていなければ None（一覧を見るだけのためには取らない）
pub async fn gpus_if_installed(dir: &Path) -> Option<Vec<Gpu>> {
    let executable = dir.join(VERSION).join(EXECUTABLE);
    if !tokio::fs::try_exists(&executable).await.unwrap_or(false) {
        return None;
    }
    list_gpus(&executable).await.ok()
}

async fn list_gpus(executable: &Path) -> anyhow::Result<Vec<Gpu>> {
    let mut command = Command::new(executable);
    command.arg("devices").stdin(std::process::Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW（コンソールの窓を出さない）
    let listing = tokio::time::timeout(Duration::from_secs(20), command.output());
    let output = listing.await.context("speech devices did not answer")??;
    anyhow::ensure!(output.status.success(), "speech devices: {}", output.status);
    Ok(parse_gpus(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows の実機（内蔵の Radeon と RTX 3070）の `speech devices` の出力
    const LISTED: &str = "\
Vulkan0    igpu  AMD Radeon(TM) Graphics, 17.0 GB, 16.1 GB free
Vulkan1    gpu   NVIDIA GeForce RTX 3070, 8.6 GB, 7.8 GB free
CPU        cpu   AMD Ryzen 7 7800X3D 8-Core Processor           , 33.4 GB, 22.9 GB free
";

    #[test]
    fn the_discrete_gpu_is_preferred_over_one_with_more_shared_memory() {
        let gpus = parse_gpus(LISTED);
        let names: Vec<&str> = gpus.iter().map(|gpu| gpu.name.as_str()).collect();
        assert_eq!(
            names,
            ["AMD Radeon(TM) Graphics", "NVIDIA GeForce RTX 3070"]
        );
        assert!(gpus[0].integrated && !gpus[1].integrated);
        assert_eq!((gpus[0].gb, gpus[1].gb), (17.0, 8.6));
        // 内蔵の GPU は、PC のメモリを分け合うので多く見える。それでも、内蔵でないほうを選ぶ
        let auto = pick(&gpus, None).unwrap();
        assert_eq!(
            (auto.id.as_str(), auto.vulkan_index()),
            ("Vulkan1", Some(1))
        );
        // 選んだ GPU があれば、それを使う。なくなっていたら、自動に戻る
        let chosen = pick(&gpus, Some("AMD Radeon(TM) Graphics")).unwrap();
        assert_eq!(chosen.vulkan_index(), Some(0));
        assert_eq!(pick(&gpus, Some("外した GPU")), Some(auto));
    }

    #[test]
    fn a_single_gpu_is_left_to_the_programs() {
        let mac = parse_gpus(
            "MTL0       gpu   Apple M5 Pro, 55.7 GB, 55.7 GB free\n\
             CPU        cpu   Apple M5 Pro, 68.7 GB, 68.7 GB free\n",
        );
        assert_eq!(mac.len(), 1);
        assert_eq!(mac[0].vulkan_index(), None);
        assert_eq!(pick(&mac, None), None);
        assert_eq!(pick(&[], Some("x")), None);
    }
}
