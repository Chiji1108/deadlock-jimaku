//! PC の中で動く文字起こし（ReazonSpeech k2）。補助プログラム `jimaku-sherpa`（`crates/sherpa`。sherpa-onnx で k2 を動かす）を
//! 別のプロセスで立て、標準入出力で頼む（DESIGN.md 9.14）。確定の文字起こしに使う。
//!
//! CPU の 1 スレッドで動き、GPU を使わない。辞書の用語をホットワードとして渡すと、カタカナの用語に強い。
//! 補助プログラムは、このリポジトリの CI（`.github/workflows/sherpa.yml`）が作ってリリースに載せたものを、
//! モデル（Apache-2.0、約 0.6 GB）は Hugging Face から取り、どちらも SHA-256 を確かめてからデータフォルダに置く（`download.rs`）。
//! ポートを開かないので、ファイアウォールの確認が出ず、ほかの PC からも頼めない。音声はこの PC から出ない。
//! プロセスは `Reazon` を落とすと終わり、アプリが終われば入力が閉じて終わる

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, bail};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::download::{self, Model, Package};

/// モデルの名前（料金の表と設定の `models.batch` に使う）
pub const REAZONSPEECH: &str = "reazonspeech-k2-v2";

/// 補助プログラムの版（`crates/sherpa/Cargo.toml`）
const VERSION: &str = "1.0.0";

/// この PC で動く配布物の名前と SHA-256（リリース `sherpa-v版` の `SHA256SUMS`）
const RELEASE: Option<(&str, &str)> = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some((
        "macos-arm64",
        "0000000000000000000000000000000000000000000000000000000000000000",
    ))
} else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
    Some((
        "macos-x64",
        "0000000000000000000000000000000000000000000000000000000000000000",
    ))
} else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    Some((
        "windows-x64",
        "0000000000000000000000000000000000000000000000000000000000000000",
    ))
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some((
        "linux-x64",
        "0000000000000000000000000000000000000000000000000000000000000000",
    ))
} else {
    None
};

const EXECUTABLE: &str = if cfg!(windows) {
    "jimaku-sherpa.exe"
} else {
    "jimaku-sherpa"
};

/// モデルの版（Hugging Face のコミット）
const REVISION: &str = "291488c8151be24d7da4bf7af26e533fad96e407";

/// モデルのファイル（補助プログラムに渡す順）と SHA-256 と大きさ。fp32 を使う。
/// int8（0.16 GB）は用語は同じだけ書けたが、文字誤り率が 5.0% から 6.9% に上がった（DESIGN.md 3 章）
const FILES: [(&str, &str, u64); 4] = [
    (
        "encoder-epoch-99-avg-1.onnx",
        "ecdb0b771e16104aaf8e579cb3c1e32fbd589eb641c5946d82b615bd366c5f96",
        592_347_848,
    ),
    (
        "decoder-epoch-99-avg-1.onnx",
        "58b18211ae06265466bfa17172dab574df94f76c8bcb61a3640c28ba860e4124",
        11_767_836,
    ),
    (
        "joiner-epoch-99-avg-1.onnx",
        "d38a81d1191c9ed6de6a1719503692e07e3e973e2364adde0abae5eaaded1174",
        10_720_115,
    ),
    (
        "tokens.txt",
        "2c3ac659818a48a0c04010e0593bbc4d7c8a24a054340b01131499c05fd52def",
        45_754,
    ),
];

/// モデルを読み込むまでの待ち時間（ファイルを取る時間は含まない）
const READY_TIMEOUT: Duration = Duration::from_secs(2 * 60);
/// 1 発話の答えを待つ時間。CPU の 1 スレッドで、長い発話でも足りる長さ
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);

/// 動いている補助プログラム
pub struct Reazon {
    /// 1 発話ずつ頼む（答えは頼んだ順に 1 行ずつ返る）。やりとりが崩れたら None にして、もう頼まない
    io: Mutex<Option<Io>>,
}

struct Io {
    /// 落とすと終わらせる（`kill_on_drop`）
    _child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
}

impl Reazon {
    /// `dir` に補助プログラムとモデルを置き（なければ取る）、立てて、モデルを読み終えるまで待つ。
    /// `hotwords` は辞書の用語（空なら渡さない）。`progress` に、モデルを取った量と全体の量（バイト）を知らせる
    pub async fn start(
        dir: &Path,
        hotwords: &[String],
        progress: &(dyn Fn(u64, u64) + Sync),
    ) -> anyhow::Result<Self> {
        let (target, sha256) = RELEASE.context(
            "jimaku-sherpa has no build for this computer (ReazonSpeech cannot run here)",
        )?;
        let url = format!(
            "https://github.com/Chiji1108/deadlock-jimaku/releases/download/sherpa-v{VERSION}/jimaku-sherpa-{VERSION}-{target}.zip"
        );
        let package = Package {
            url: &url,
            sha256,
            folder: VERSION,
            file: EXECUTABLE,
        };
        let executable = download::installed(dir, &package).await?;
        let models = dir.join("models").join(REVISION);
        let total: u64 = FILES.iter().map(|(_, _, bytes)| bytes).sum();
        let mut before = 0;
        let mut command = Command::new(&executable);
        for (file, sha256, bytes) in FILES {
            let url = format!(
                "https://huggingface.co/reazon-research/reazonspeech-k2-v2/resolve/{REVISION}/{file}"
            );
            let model = Model {
                url: &url,
                sha256,
                file,
            };
            let each = |done, _| progress(before + done, total);
            command.arg(download::model(&models, &model, &each).await?);
            before += bytes;
        }
        let hotwords = lines(hotwords);
        if !hotwords.is_empty() {
            let file = dir.join("hotwords.txt");
            tokio::fs::write(&file, hotwords).await?;
            command.arg(file);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW（コンソールの窓を出さない）
        let mut child = command.spawn().context("cannot start jimaku-sherpa")?;
        let stdin = child.stdin.take().context("stdin")?;
        let mut lines = BufReader::new(child.stdout.take().context("stdout")?).lines();
        let ready = tokio::time::timeout(READY_TIMEOUT, lines.next_line())
            .await
            .context("jimaku-sherpa did not get ready")??
            .context("jimaku-sherpa exited")?;
        let ready: serde_json::Value = serde_json::from_str(&ready)?;
        anyhow::ensure!(ready["ready"] == true, "jimaku-sherpa: {ready}");
        tracing::info!("jimaku-sherpa {VERSION} serves {REAZONSPEECH}");
        let io = Io {
            _child: child,
            stdin,
            lines,
        };
        Ok(Self {
            io: Mutex::new(Some(io)),
        })
    }

    /// 発話（16kHz mono）を文字にする
    pub async fn transcribe(&self, samples: &[f32]) -> anyhow::Result<String> {
        let mut io = self.io.lock().await;
        let Some(running) = io.as_mut() else {
            bail!("jimaku-sherpa stopped");
        };
        let answer = tokio::time::timeout(ANSWER_TIMEOUT, ask(running, samples)).await;
        match answer {
            Ok(Ok(text)) => Ok(text),
            // 答えの行がずれると、次の発話に前の答えが付くので、もう頼まない（プロセスも終わる）
            Ok(Err(e)) => {
                *io = None;
                Err(e)
            }
            Err(_) => {
                *io = None;
                bail!("jimaku-sherpa did not answer")
            }
        }
    }
}

async fn ask(io: &mut Io, samples: &[f32]) -> anyhow::Result<String> {
    let count = u32::try_from(samples.len())?;
    let mut frame = Vec::with_capacity(4 + samples.len() * 4);
    frame.extend_from_slice(&count.to_le_bytes());
    frame.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
    io.stdin.write_all(&frame).await?;
    io.stdin.flush().await?;
    let line = io
        .lines
        .next_line()
        .await?
        .context("jimaku-sherpa exited")?;
    let answer: serde_json::Value = serde_json::from_str(&line)?;
    answer["text"]
        .as_str()
        .map(str::to_owned)
        .context("jimaku-sherpa answered without text")
}

/// ホットワードのファイル（1 行に 1 語）。改行を含む語は、1 語のまま渡せないので除く
fn lines(hotwords: &[String]) -> String {
    hotwords
        .iter()
        .map(|word| word.trim())
        .filter(|word| !word.is_empty() && !word.contains(['\n', '\r']))
        .map(|word| format!("{word}\n"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotwords_are_one_per_line() {
        let words = ["ヘイズ", " インファーナス ", "", "二行\nの語"].map(String::from);
        assert_eq!(lines(&words), "ヘイズ\nインファーナス\n");
        assert_eq!(lines(&[]), "");
    }

    /// 本物の補助プログラムとモデル（約 0.6 GB）で、合成音声を文字起こしする。用語を渡す。
    /// リリースの前は、`REAZON_TEST_DIR/1.0.0/` に手元で作った補助プログラム（`crates/sherpa`）を置いておく。
    /// ふだんは動かさない: `REAZON_TEST_DIR=… cargo test -p overlays-core reazon_writes -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "downloads jimaku-sherpa and the model"]
    async fn reazon_writes_the_terms_it_was_told() {
        let dir = std::env::var("REAZON_TEST_DIR").expect("REAZON_TEST_DIR");
        let words = ["ヘイズ", "スモークボム"].map(String::from);
        let reazon = Reazon::start(Path::new(&dir), &words, &|_, _| {})
            .await
            .unwrap();
        let wav = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../experiments/data/tts/aya/001.wav"
        );
        let mut reader = hound::WavReader::open(wav).unwrap();
        let samples: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / f32::from(i16::MAX))
            .collect();
        let started = std::time::Instant::now();
        let text = reazon.transcribe(&samples).await.unwrap();
        println!("{text}（{:?}）", started.elapsed());
        assert!(
            text.contains("ヘイズ") && text.contains("スモークボム"),
            "{text}"
        );
        // 続けて頼んでも、答えがずれない
        assert_eq!(reazon.transcribe(&samples).await.unwrap(), text);
    }
}
