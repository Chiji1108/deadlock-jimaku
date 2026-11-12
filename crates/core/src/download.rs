//! あとから取ってくる部品（PC の中の文字起こし）を、データフォルダに置く。
//!
//! 配布物（zip）を取り、SHA-256 を確かめてから広げる。版ごとのフォルダに置くので、版を上げると取り直す。
//! インストーラーに入れると、アップデートのたびに取り直すことになる大きさのものに使う

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::ensure;
use sha2::{Digest, Sha256};

/// ふだんの API の待ち時間（60 秒）では足りない回線がある（配布物は数十 MB）
const TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// 取ってくる配布物
pub struct Package<'a> {
    pub url: &'a str,
    /// zip の SHA-256（16 進）
    pub sha256: &'a str,
    /// 広げて置くフォルダの名前（版を入れる）
    pub folder: &'a str,
    /// 広げたフォルダの中の、これがあれば置いてあるとみなすファイル
    pub file: &'a str,
}

/// `dir` の下に置いてある配布物のファイル（`package.file`）。なければ取って置く
pub async fn installed(dir: &Path, package: &Package<'_>) -> anyhow::Result<PathBuf> {
    let folder = dir.join(package.folder);
    let file = folder.join(package.file);
    if tokio::fs::try_exists(&file).await? {
        return Ok(file);
    }
    tracing::info!("downloading {}", package.url);
    let res = crate::http().get(package.url).timeout(TIMEOUT);
    let res = res.send().await?.error_for_status()?;
    let archive = res.bytes().await?;
    let digest: String = Sha256::digest(&archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    ensure!(
        digest == package.sha256,
        "{} has a wrong checksum",
        package.url
    );
    // 途中で止まっても半端なフォルダを使わないよう、別の名前に広げてから置き換える。広げている間に
    // もう一度頼まれても（広げる処理は止められない）ぶつからないよう、名前は毎回変える
    static UNPACKED: AtomicU32 = AtomicU32::new(0);
    let count = UNPACKED.fetch_add(1, Ordering::Relaxed);
    let from = dir.join(format!(
        "{}.{}-{count}.tmp",
        package.folder,
        std::process::id()
    ));
    let installed = file.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        std::fs::create_dir_all(&from)?;
        zip::ZipArchive::new(Cursor::new(archive))?.extract(&from)?;
        let renamed = std::fs::rename(&from, &folder);
        // 先に置いた処理があれば、それを使う
        if renamed.is_err() && installed.exists() {
            let _ = std::fs::remove_dir_all(&from);
            return Ok(());
        }
        Ok(renamed?)
    })
    .await??;
    Ok(file)
}
