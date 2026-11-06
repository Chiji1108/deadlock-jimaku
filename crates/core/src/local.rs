//! ローカル AI のモード（DESIGN.md 9.6）。途中の字幕と途中の翻訳を、PC の中の補助プログラムで作る。
//!
//! 補助プログラム（`crates/local-ai`）は実行環境が大きいので、アプリに入れず、使う人だけ初回に
//! ダウンロードする。別のプロセスにすると、使わない人のメモリも増えない

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use flate2::read::GzDecoder;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;

use crate::settings::Settings;
use crate::store::Store;
use crate::types::{Glossary, LiveState, LiveStatus, LiveText, Token, Translations};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "https://github.com/Chiji1108/deadlock-jimaku/releases/download";
/// 開発中は、手元でビルドした補助プログラムを使う（`mise run local`）
const DEV_DIR: &str = "JIMAKU_LOCAL_DIR";
/// 補助プログラムのアーカイブの SHA-256。CI がビルドのときに埋め込む（手元のビルドにはないので、
/// `JIMAKU_LOCAL_DIR` を使う）
const ARCHIVE_SHA256: Option<&str> = if cfg!(windows) {
    option_env!("JIMAKU_LOCAL_SHA256_WINDOWS_X86_64")
} else if cfg!(target_os = "linux") {
    option_env!("JIMAKU_LOCAL_SHA256_LINUX_X86_64")
} else if cfg!(target_arch = "aarch64") {
    option_env!("JIMAKU_LOCAL_SHA256_MACOS_AARCH64")
} else {
    option_env!("JIMAKU_LOCAL_SHA256_MACOS_X86_64")
};
/// 遅い回線でも、大きなファイルを受け取れるようにする
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// SenseVoice が聞き取れる言語
const LANGUAGES: [&str; 5] = ["ja", "en", "zh", "ko", "yue"];
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const AUDIO: u8 = 1;
const CUT: u8 = 2;

pub enum Input {
    Audio(Vec<f32>),
    /// 発話が閉じた
    Cut,
}

/// 用語の表。翻訳の前に英語の名前へ置き換える
pub fn terms(glossary: &Glossary) -> Vec<(String, String)> {
    glossary
        .entities
        .values()
        .filter_map(|e| Some((e.names.get("ja")?.clone(), e.names.get("en")?.clone())))
        .collect()
}

pub async fn run(
    store: Arc<Store>,
    settings: Settings,
    terms: Vec<(String, String)>,
    mut input: mpsc::UnboundedReceiver<Input>,
) {
    show(&store, LiveState::Preparing, None);
    if let Err(e) = session(&store, &settings, &terms, &mut input).await {
        tracing::warn!("local ai: {e:#}");
        show(
            &store,
            LiveState::Failed,
            Some(format!("ローカル AI を起動できません: {e}")),
        );
    }
}

fn show(store: &Store, state: LiveState, detail: Option<String>) {
    store.set_live_status(Some(LiveStatus { state, detail }));
}

async fn session(
    store: &Arc<Store>,
    settings: &Settings,
    terms: &[(String, String)],
    input: &mut mpsc::UnboundedReceiver<Input>,
) -> anyhow::Result<()> {
    let language = settings.mic_language.split('-').next().unwrap_or_default();
    if !LANGUAGES.contains(&language) {
        bail!("この言語には対応していません");
    }
    let root = store.data_dir().join("local-ai");
    tokio::fs::create_dir_all(&root).await?;
    let dir = install(store, &root).await?;
    let terms_path = root.join("terms.json");
    tokio::fs::write(&terms_path, serde_json::to_vec(terms)?).await?;
    let mut command = Command::new(dir.join(binary()));
    command
        .arg(root.join("models"))
        .arg(language)
        .arg(&terms_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // コンソールの窓を出さない（全画面のゲームからフォーカスを奪う）
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn().context("start")?;
    let stdin = child.stdin.take().context("stdin")?;
    // 書き込みで止まっても、出力を読み続ける
    let (writer, rx) = mpsc::unbounded_channel();
    tokio::spawn(write_input(stdin, rx));
    let mut lines = BufReader::new(child.stdout.take().context("stdout")?).lines();
    let translate = settings.translate_to.iter().any(|l| l == "en");
    // 準備が終わるまでの音声は渡さない（古い発話の字幕が今の発話に載る）
    let mut ready = false;
    loop {
        tokio::select! {
            next = input.recv() => {
                let Some(next) = next else { return Ok(()) };
                if ready {
                    let _ = writer.send(next);
                }
            }
            line = lines.next_line() => {
                let line = line?.context("stopped")?;
                let message: Value = serde_json::from_str(&line)?;
                ready |= message["type"] == "ready";
                on_line(store, &message, translate);
            }
        }
    }
}

async fn write_input(mut stdin: ChildStdin, mut rx: mpsc::UnboundedReceiver<Input>) {
    while let Some(input) = rx.recv().await {
        if send(&mut stdin, input).await.is_err() {
            return;
        }
    }
}

async fn send(stdin: &mut ChildStdin, input: Input) -> std::io::Result<()> {
    match input {
        Input::Audio(samples) => {
            let mut frame = Vec::with_capacity(5 + samples.len() * 4);
            frame.push(AUDIO);
            frame.extend_from_slice(&((samples.len() * 4) as u32).to_le_bytes());
            frame.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
            stdin.write_all(&frame).await
        }
        Input::Cut => stdin.write_all(&[CUT]).await,
    }
}

fn on_line(store: &Store, message: &Value, translate: bool) {
    match message["type"].as_str() {
        Some("progress") => {
            let percent = message["loaded"].as_u64().unwrap_or(0) * 100
                / message["total"].as_u64().unwrap_or(1).max(1);
            show(
                store,
                LiveState::Preparing,
                Some(format!(
                    "モデルをダウンロードしています（{}%）",
                    percent.min(100)
                )),
            );
        }
        Some("ready") => show(store, LiveState::Listening, None),
        Some("text") => {
            let translations: Translations = message["translation"]
                .as_str()
                .filter(|text| translate && !text.is_empty())
                .map(|text| {
                    let tokens = vec![Token::Text {
                        text: text.to_owned(),
                        ruby: None,
                    }];
                    Translations::from([("en".to_owned(), tokens)])
                })
                .unwrap_or_default();
            store.send_live(LiveText {
                text: message["text"].as_str().unwrap_or_default().to_owned(),
                translations,
            });
        }
        _ => {}
    }
}

fn binary() -> &'static str {
    if cfg!(windows) {
        "jimaku-local.exe"
    } else {
        "jimaku-local"
    }
}

/// 補助プログラムのあるフォルダ。なければリリースからダウンロードする
async fn install(store: &Store, root: &Path) -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os(DEV_DIR) {
        return Ok(dir.into());
    }
    let dir = root.join(VERSION);
    if dir.join(binary()).exists() {
        return Ok(dir);
    }
    show(
        store,
        LiveState::Preparing,
        Some("ローカル AI をダウンロードしています".into()),
    );
    let archive = format!(
        "jimaku-local-{}-{}.tar.gz",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let expected = ARCHIVE_SHA256.context("no checksum in this build")?;
    let url = format!("{RELEASES}/v{VERSION}/{archive}");
    let bytes = crate::http()
        .get(&url)
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    if !checksum_matches(&bytes, expected) {
        bail!("{archive}: the checksum does not match");
    }
    let extract_to = dir.clone();
    tokio::task::spawn_blocking(move || extract(&bytes, &extract_to)).await??;
    Ok(dir)
}

fn checksum_matches(bytes: &[u8], expected: &str) -> bool {
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    digest == expected.trim()
}

/// 途中で止まっても半端なフォルダを残さないよう、できてから置く
fn extract(bytes: &[u8], dir: &Path) -> anyhow::Result<()> {
    let partial = dir.with_extension("partial");
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial)?;
    tar::Archive::new(GzDecoder::new(bytes)).unpack(&partial)?;
    std::fs::rename(partial, dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_detects_a_changed_file() {
        let sha256 = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(checksum_matches(b"hello", sha256));
        assert!(!checksum_matches(b"hello!", sha256));
    }

    #[test]
    fn extracts_the_archive_into_place() {
        let dir = tempfile::tempdir().unwrap();
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(2);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, "jimaku-local", &b"hi"[..])
            .unwrap();
        let bytes = archive.into_inner().unwrap().finish().unwrap();
        let target = dir.path().join("0.3.0");
        extract(&bytes, &target).unwrap();
        assert_eq!(std::fs::read(target.join("jimaku-local")).unwrap(), b"hi");
    }
}
