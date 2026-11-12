pub mod access;
pub mod annotate;
pub mod assets;
pub mod audio;
pub mod chrome;
pub mod decide;
pub mod demo;
pub mod download;
pub mod enrich;
pub mod finish;
pub mod fonts;
pub mod furigana;
pub mod lang;
pub mod llm;
pub mod local;
pub mod markup;
pub mod mt;
pub mod rank;
pub mod reading;
pub mod runtime;
pub mod secrets;
pub mod segment;
pub mod server;
pub mod settings;
pub mod share;
pub mod store;
pub mod stt;
pub mod subtitles;
pub mod terms;
pub mod types;
pub mod usage;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// オーバーレイを配信するポート。設定化は Phase 5
pub const DEFAULT_PORT: u16 = 4510;

/// app/tauri.conf.json の identifier と揃える（Tauri の app_data_dir と同じ場所になる）
pub const IDENTIFIER: &str = "jp.deadlock.jimaku";

/// 開発のときに、使っているアプリとぶつからないよう、別の場所で起動する（README の「開発」）。
/// `DEADLOCK_JIMAKU_DATA_DIR` を付けると、設定と辞書をそのフォルダに置き、API キーも別の名前で持つ
/// （使っているアプリのキーを読まない）。ポートは `DEADLOCK_JIMAKU_PORT` で替える
const DATA_DIR_ENV: &str = "DEADLOCK_JIMAKU_DATA_DIR";
const PORT_ENV: &str = "DEADLOCK_JIMAKU_PORT";

/// 操作パネルとオーバーレイを配るポート
pub fn port() -> u16 {
    let port = std::env::var(PORT_ENV).ok().and_then(|p| p.parse().ok());
    port.unwrap_or(DEFAULT_PORT)
}

/// 開発のときに指定した、データの置き場所
pub fn data_dir_override() -> Option<PathBuf> {
    std::env::var_os(DATA_DIR_ENV).map(Into::into)
}

pub fn default_data_dir() -> PathBuf {
    data_dir_override().unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(IDENTIFIER)
    })
}

/// API キーを OS に預けるときの名前。別の場所で起動したときは、使っているアプリのものと分ける
pub fn keyring_service() -> String {
    match data_dir_override() {
        Some(_) => format!("{IDENTIFIER}.dev"),
        None => IDENTIFIER.to_owned(),
    }
}

/// 外部 API（OpenAI・deadlock-api）に使う HTTP クライアント。接続と TLS を使い回す。
/// 応答が返らないまま固まらないよう、既定のタイムアウトを付ける（長くかかるものはリクエストごとに上書きする）
pub fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("valid client config")
    })
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_millis() as u64
}

/// 16 進の SHA-256
pub(crate) fn sha256(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 途中で止まっても壊れたファイルが残らないよう、一時ファイルに書いてから置き換える
pub async fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(path.parent().expect("has parent")).await?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}
