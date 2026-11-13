//! PC の中で動く文字起こし（Whisper）。whisper.cpp（MIT）の `whisper-server` を別のプロセスで立て、
//! OpenAI と同じ形の口で頼む（DESIGN.md 9.10）。確定の文字起こしに使う。
//!
//! `whisper-server` は、このリポジトリの CI（`.github/workflows/whisper.yml`）が作ってリリースに載せたものを、
//! モデル（MIT）は Hugging Face から取り、どちらも SHA-256 を確かめてからデータフォルダに置く（`download.rs`）。
//! 音声はこの PC から出ない。プロセスは字幕を止めると終わる

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use tokio::process::Command;

use crate::download::{self, Model, Package};
use crate::sidecar::{self, Sidecar};

/// モデルの名前（料金の表と設定の `models.batch` に使う）
pub const TURBO: &str = "whisper-large-v3-turbo";
pub const LARGE: &str = "whisper-large-v3";

/// どのモデルで書くか
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// large-v3-turbo を小さくしたもの（q5_0、0.57 GB）。元と精度も速さも変わらなかった（DESIGN.md 3 章）
    Turbo,
    /// large-v3 を小さくしたもの（q5_0、1.1 GB）。turbo より用語に強く、約 2 倍遅い
    Large,
}

impl Size {
    pub fn name(self) -> &'static str {
        match self {
            Self::Turbo => TURBO,
            Self::Large => LARGE,
        }
    }

    /// whisper.cpp の作者が Hugging Face に置いているファイル（版を固定する）
    fn model(self) -> Model<'static> {
        match self {
            Self::Turbo => Model {
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin",
                sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
                file: "ggml-large-v3-turbo-q5_0.bin",
            },
            Self::Large => Model {
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-q5_0.bin",
                sha256: "d75795ecff3f83b5faa89d1900604ad8c780abd5739fae406de19f23ecd98ad1",
                file: "ggml-large-v3-q5_0.bin",
            },
        }
    }
}

const VERSION: &str = "1.9.5";

/// この PC で動く配布物の名前と SHA-256（リリースの `SHA256SUMS`）。Intel の Mac の配布物はない
const RELEASE: Option<(&str, &str)> = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some((
        "macos-arm64-metal",
        "57a20eaef496b8bd6ca23b7236f1e749d34b51b8962153a8dc26d7b1d54674c2",
    ))
} else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    Some((
        "windows-x64-vulkan",
        "276df5f3eaffcbc8fdb92f8b5d8426161aa36de3777974e7fcd4fe921a1f8099",
    ))
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some((
        "linux-x64-cpu",
        "87ad9d8b81753d49a13115b468e1d0bb0710f5e1492e19cdb0e8b4510c8576f6",
    ))
} else {
    None
};

const EXECUTABLE: &str = if cfg!(windows) {
    "whisper-server.exe"
} else {
    "whisper-server"
};

/// モデルを GPU に載せるまでの待ち時間（ファイルを取る時間は含まない）
const READY_TIMEOUT: Duration = Duration::from_secs(5 * 60);

static RUNNING: Sidecar = Sidecar::new("whisper.cpp");

/// whisper.cpp を終わらせる
pub fn stop() {
    RUNNING.stop();
}

/// 動いている whisper.cpp のサーバー
pub struct Whisper {
    port: u16,
}

impl Drop for Whisper {
    fn drop(&mut self) {
        stop();
    }
}

impl Whisper {
    /// `dir` に whisper.cpp とモデルを置き（なければ取る）、サーバーを立てて、頼めるようになるまで待つ。
    /// `progress` に、モデルを取った量と全体の量（バイト）を知らせる
    pub async fn start(
        dir: &Path,
        size: Size,
        progress: &(dyn Fn(u64, u64) + Sync),
    ) -> anyhow::Result<Self> {
        let (target, sha256) = RELEASE
            .context("whisper.cpp has no build for this computer (Whisper cannot run here)")?;
        let url = format!(
            "https://github.com/Chiji1108/deadlock-jimaku/releases/download/whisper-v{VERSION}/whisper-{VERSION}-{target}.zip"
        );
        let package = Package {
            url: &url,
            sha256,
            folder: VERSION,
            file: EXECUTABLE,
        };
        let executable = download::installed(dir, &package).await?;
        let model = download::model(&dir.join("models"), &size.model(), progress).await?;
        let port = sidecar::free_port()?;
        let mut command = Command::new(&executable);
        command
            .arg("--model")
            .arg(&model)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            // OpenAI と同じパスで受ける（言語と用語は、頼むたびに渡す）
            .args(["--inference-path", "/v1/audio/transcriptions"]);
        // ここから先で失敗しても、待っている間に捨てられても、プロセスを残さない
        let whisper = Self { port };
        RUNNING.serve(command, port, READY_TIMEOUT).await?;
        tracing::info!(
            "whisper.cpp {VERSION} serves {} on port {port}",
            size.name()
        );
        Ok(whisper)
    }

    /// OpenAI の `/v1/audio/transcriptions` と同じ形の口に送る
    pub fn transcriptions(&self) -> reqwest::RequestBuilder {
        let url = format!("http://127.0.0.1:{}/v1/audio/transcriptions", self.port);
        // 長い発話を CPU だけで認識しても足りる長さ
        sidecar::client().post(url).timeout(Duration::from_secs(60))
    }
}
