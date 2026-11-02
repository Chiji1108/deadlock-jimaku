pub mod assets;
pub mod demo;
pub mod lang;
pub mod server;
pub mod settings;
pub mod store;
pub mod types;

use std::path::PathBuf;

/// オーバーレイを配信するポート。設定化は Phase 5
pub const DEFAULT_PORT: u16 = 4510;

/// app/tauri.conf.json の identifier と揃える（Tauri の app_data_dir と同じ場所になる）
pub const IDENTIFIER: &str = "dev.deadlock-stream-overlays.app";

pub fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(IDENTIFIER)
}
