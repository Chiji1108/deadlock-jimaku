// リリースビルドで Windows のコンソールウィンドウを出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::{Ipv4Addr, SocketAddr};

use overlays_core::{
    DEFAULT_PORT, assets, audio, rank, server,
    settings::{self, Settings},
    store::Store,
    subtitles,
};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::net::TcpListener;

fn main() {
    tracing_subscriber::fmt::init();

    tauri::Builder::default()
        .setup(|app| {
            // 設定・API キーの保存と、操作パネルからの変更・開始・停止は Phase 5
            let settings = Settings {
                rank_account: settings::account_id(
                    std::env::var("DEADLOCK_ACCOUNT_ID").ok().as_deref(),
                ),
                ..Settings::default()
            };
            // Mistral は文字起こし、OpenAI は翻訳と辞書の漢字名の読み
            let api_key = std::env::var("MISTRAL_API_KEY").ok();
            let openai_key = std::env::var("OPENAI_API_KEY").ok();
            let store = Store::new(settings);
            tauri::async_runtime::spawn(rank::run_configured(store.clone()));
            tauri::async_runtime::spawn(assets::load_into(
                store.clone(),
                app.path().app_data_dir()?.join("assets"),
                openai_key.clone(),
            ));
            match api_key {
                Some(api_key) => {
                    let store = store.clone();
                    let input = audio::Input::Mic(store.settings().mic_device.clone());
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = subtitles::run(store, input, api_key, openai_key).await {
                            tracing::error!("transcription stopped: {e:#}");
                        }
                    });
                }
                None => tracing::warn!("MISTRAL_API_KEY is not set; transcription is off"),
            }
            // ウィンドウを開く前に bind して、ポートが使えないことを起動時に検出する
            let listener = tauri::async_runtime::block_on(TcpListener::bind(SocketAddr::from((
                Ipv4Addr::LOCALHOST,
                DEFAULT_PORT,
            ))))?;
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server::serve(listener, store).await {
                    tracing::error!("server stopped: {e}");
                }
            });

            WebviewWindowBuilder::new(app, "main", control_url()?)
                .title("Deadlock Stream Overlays")
                .inner_size(960.0, 720.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 操作パネルは OBS のドックと同じく localhost の画面を開く（IPC は使わない）。
/// 開発中は Vite（devUrl）から開き、/api と /ws は Vite が axum にプロキシする。
fn control_url() -> Result<WebviewUrl, url::ParseError> {
    if cfg!(debug_assertions) {
        Ok(WebviewUrl::App("control.html".into()))
    } else {
        Ok(WebviewUrl::External(
            format!("http://127.0.0.1:{DEFAULT_PORT}/").parse()?,
        ))
    }
}
