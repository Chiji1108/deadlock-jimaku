//! あとから取ってくる部品（PC の中の文字起こしと LLM）を、データフォルダに置く。
//!
//! 配布物（zip か tar.gz）を取り、SHA-256 を確かめてから広げる。版ごとのフォルダに置くので、版を上げると取り直す。
//! インストーラーに入れると、アップデートのたびに取り直すことになる大きさのものに使う

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::ensure;
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// ふだんの API の待ち時間（60 秒）では足りない回線がある（配布物は数十 MB）
const TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// 取ってくる配布物
pub struct Package<'a> {
    pub url: &'a str,
    /// 配布物の SHA-256（16 進）
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
    ensure!(
        hex(Sha256::digest(&archive)) == package.sha256,
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
    let tar = package.url.ends_with(".tar.gz");
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        std::fs::create_dir_all(&from)?;
        if tar {
            let gz = flate2::read::GzDecoder::new(Cursor::new(archive));
            tar::Archive::new(gz).unpack(&from)?;
        } else {
            zip::ZipArchive::new(Cursor::new(archive))?.extract(&from)?;
        }
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

fn hex(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 取ってくるモデルのファイル（数 GB）
pub struct Model<'a> {
    pub url: &'a str,
    /// SHA-256（16 進）
    pub sha256: &'a str,
    /// 置くファイルの名前
    pub file: &'a str,
}

/// 大きいファイルを取るクライアント。全体の時間は区切らず、流れが止まったら諦める
fn slow_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .build()
            .expect("valid client config")
    })
}

/// `dir` の下に置いてあるモデルのファイル。なければ取って、SHA-256 を確かめてから置く。
/// 途中で止まったら、次は続きから取る（`.part` のファイルに書きためる）。`progress` に、取った量と全体の量（バイト）を知らせる
pub async fn model(
    dir: &Path,
    model: &Model<'_>,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> anyhow::Result<PathBuf> {
    let file = dir.join(model.file);
    if tokio::fs::try_exists(&file).await? {
        return Ok(file);
    }
    tokio::fs::create_dir_all(dir).await?;
    let part = dir.join(format!("{}.part", model.file));
    // 前に取った分。ハッシュは全体で確かめるので、読み直す
    let mut hasher = Sha256::new();
    let mut have = 0u64;
    if let Ok(mut before) = tokio::fs::File::open(&part).await {
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let n = before.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            have += n as u64;
        }
    }
    tracing::info!("downloading {} from byte {have}", model.url);
    let mut request = slow_client().get(model.url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let res = request.send().await?;
    // 全部取ってあった（続きがない）なら、確かめるだけ
    if res.status() != StatusCode::RANGE_NOT_SATISFIABLE {
        let mut res = res.error_for_status()?;
        // 続きから返さないサーバーなら、最初から取り直す
        let resumed = res.status() == StatusCode::PARTIAL_CONTENT;
        if !resumed {
            hasher = Sha256::new();
        }
        let mut out = tokio::fs::OpenOptions::new();
        out.create(true).write(true);
        let out = if resumed {
            out.append(true)
        } else {
            out.truncate(true)
        };
        let mut out = out.open(&part).await?;
        let mut done = if resumed { have } else { 0 };
        let total = done + res.content_length().unwrap_or(0);
        while let Some(chunk) = res.chunk().await? {
            hasher.update(&chunk);
            out.write_all(&chunk).await?;
            done += chunk.len() as u64;
            progress(done, total);
        }
        out.flush().await?;
    }
    if hex(hasher.finalize()) != model.sha256 {
        // 壊れたものを続きから取っても直らない
        let _ = tokio::fs::remove_file(&part).await;
        anyhow::bail!("{} has a wrong checksum", model.url);
    }
    tokio::fs::rename(&part, &file).await?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 途中まで取ってあるファイルは、続きだけを取る。ハッシュが合わなければ置かない
    #[tokio::test]
    async fn a_model_resumes_and_is_checked() {
        use axum::http::{HeaderMap, header::RANGE};
        const BODY: &[u8] = b"0123456789abcdef";
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/model.bin", listener.local_addr().unwrap());
        let serve = |headers: HeaderMap| async move {
            let from = headers.get(RANGE).and_then(|range| {
                let range = range.to_str().ok()?.strip_prefix("bytes=")?;
                range.strip_suffix('-')?.parse::<usize>().ok()
            });
            match from {
                Some(from) if from >= BODY.len() => (StatusCode::RANGE_NOT_SATISFIABLE, &[][..]),
                Some(from) => (StatusCode::PARTIAL_CONTENT, &BODY[from..]),
                None => (StatusCode::OK, BODY),
            }
        };
        let router = axum::Router::new().route("/model.bin", axum::routing::get(serve));
        tokio::spawn(async move { axum::serve(listener, router).await });
        let sha256 = hex(Sha256::digest(BODY));
        let dir = tempfile::tempdir().unwrap();
        let wanted = |file, sha256| Model {
            url: &url,
            sha256,
            file,
        };

        std::fs::write(dir.path().join("resumed.bin.part"), &BODY[..6]).unwrap();
        let seen = std::sync::Mutex::new(Vec::new());
        let progress = |done, total| seen.lock().unwrap().push((done, total));
        let file = model(dir.path(), &wanted("resumed.bin", &sha256), &progress).await;
        assert_eq!(std::fs::read(file.unwrap()).unwrap(), BODY);
        assert_eq!(seen.lock().unwrap().last(), Some(&(16, 16)));

        // 全部取ってあって、置く前に止まったもの
        std::fs::write(dir.path().join("whole.bin.part"), BODY).unwrap();
        let file = model(dir.path(), &wanted("whole.bin", &sha256), &|_, _| {}).await;
        assert_eq!(std::fs::read(file.unwrap()).unwrap(), BODY);

        let wrong = model(dir.path(), &wanted("wrong.bin", "00"), &|_, _| {}).await;
        assert!(wrong.is_err());
        assert!(!dir.path().join("wrong.bin").exists());
        assert!(!dir.path().join("wrong.bin.part").exists());
    }
}
