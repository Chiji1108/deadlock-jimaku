// リリースビルドで Windows のコンソールウィンドウを出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::{Ipv4Addr, SocketAddr};

use overlays_core::{DEFAULT_PORT, runtime, server, settings, store::Store};
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;
use tokio::net::TcpListener;

fn main() {
    tracing_subscriber::fmt::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
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

            // 開発中は確かめない（リリースの版と比べても意味がない）
            if !cfg!(debug_assertions) {
                let app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = update(app).await {
                        tracing::warn!("update: {e}");
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// 新しい版があれば聞いて、入れ替えて再起動する（DESIGN.md 13.5）。
/// 更新情報は GitHub の最新のリリースの latest.json で、署名を公開鍵（tauri.conf.json）で確かめる
async fn update(app: AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let Some(update) = app.updater()?.check().await? else {
        return Ok(());
    };
    tracing::info!("update available: {}", update.version);
    let dialog = app
        .dialog()
        .message(format!(
            "新しい版（{}）があります。今の版は {} です。\n更新して再起動しますか？（字幕は一度止まります）",
            update.version, update.current_version
        ))
        .title("デドロ字幕の更新")
        // 操作パネルの前に出す（親がないと、Windows ではウィンドウの裏に隠れることがある）
        .parent(&app.get_webview_window("main").ok_or("no main window")?)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "更新する".into(),
            "あとで".into(),
        ));
    // 答えを待つ間、非同期のワーカーを塞がない
    if !tauri::async_runtime::spawn_blocking(move || dialog.blocking_show()).await? {
        return Ok(());
    }
    update.download_and_install(|_, _| {}, || {}).await?;
    app.restart();
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
