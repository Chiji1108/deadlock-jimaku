// リリースビルドで Windows のコンソールウィンドウを出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::{Ipv4Addr, SocketAddr};

use overlays_core::{DEFAULT_PORT, runtime, server, settings, store::Store};
use tauri::webview::NewWindowResponse;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::net::TcpListener;

fn main() {
    tracing_subscriber::fmt::init();

    tauri::Builder::default()
        .setup(|app| {
            // 設定は操作パネルから変えて保存する。API キーは OS の資格情報ストア
            let data_dir = app.path().app_data_dir()?;
            let settings = settings::load(&data_dir.join(settings::FILE));
            let store = Store::new(settings, data_dir);
            tauri::async_runtime::spawn(runtime::run(
                store.clone(),
                runtime::Options { input_file: None },
            ));
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
                .title("デドロ字幕")
                // Windows ではファイルを落とす処理があるとページの中のドラッグが働かない。
                // 「OBS にドラッグして追加」を使うので切る（このアプリはファイルを落として使わない）
                .disable_drag_drop_handler()
                .inner_size(960.0, 720.0)
                // 外部へのリンク（API キーの作成ページなど）はふだんのブラウザで開く
                .on_new_window(|url, _| {
                    if let Err(e) = tauri_plugin_opener::open_url(url.as_str(), None::<&str>) {
                        tracing::warn!("cannot open {url}: {e}");
                    }
                    NewWindowResponse::Deny
                })
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
