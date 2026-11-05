//! ローカル AI のモード（DESIGN.md 9.6）。途中の字幕と途中の翻訳を、PC の中の補助プログラムで作る。
//!
//! 補助プログラム（`crates/local-ai`）は実行環境が大きいので、アプリに入れず、使う人だけ初回に
//! ダウンロードする。別のプロセスにすると、使わない人のメモリも増えない

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::Context;
use flate2::read::GzDecoder;
use serde_json::Value;
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
    let root = store.data_dir().join("local-ai");
    let dir = install(store, &root).await?;
    let terms_path = root.join("terms.json");
    tokio::fs::write(&terms_path, serde_json::to_vec(terms)?).await?;
    let mut child = Command::new(dir.join(binary()))
        .arg(root.join("models"))
        .arg(&settings.mic_language)
        .arg(&terms_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("start")?;
    let mut stdin = child.stdin.take().context("stdin")?;
    let mut lines = BufReader::new(child.stdout.take().context("stdout")?).lines();
    let translate = settings.translate_to.iter().any(|l| l == "en");
    loop {
        tokio::select! {
            next = input.recv() => {
                let Some(next) = next else { return Ok(()) };
                send(&mut stdin, next).await?;
            }
            line = lines.next_line() => {
                let line = line?.context("stopped")?;
                on_line(store, &serde_json::from_str(&line)?, translate);
            }
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
    let url = format!("{RELEASES}/v{VERSION}/{archive}");
    let bytes = crate::http()
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let extract_to = dir.clone();
    tokio::task::spawn_blocking(move || extract(&bytes, &extract_to)).await??;
    Ok(dir)
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
