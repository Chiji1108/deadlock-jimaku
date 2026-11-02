pub mod assets;
pub mod audio;
pub mod demo;
pub mod enrich;
pub mod furigana;
pub mod lang;
pub mod llm;
pub mod markup;
pub mod rank;
pub mod runtime;
pub mod secrets;
pub mod server;
pub mod settings;
pub mod store;
pub mod stt;
pub mod subtitles;
pub mod types;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// オーバーレイを配信するポート。設定化は Phase 5
pub const DEFAULT_PORT: u16 = 4510;

/// app/tauri.conf.json の identifier と揃える（Tauri の app_data_dir と同じ場所になる）
pub const IDENTIFIER: &str = "dev.deadlock-stream-overlays.app";

pub fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(IDENTIFIER)
}

/// 外部 API（Mistral・OpenAI・deadlock-api）に使う HTTP クライアント。接続と TLS を使い回す。
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
