// リリースビルドで Windows のコンソールウィンドウを出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_os = "linux"))]
mod tray;

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use overlays_core::{DEFAULT_PORT, runtime, server, settings, store::Store};
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;
use tauri_plugin_window_state::StateFlags;
use tokio::net::TcpListener;

const UPDATE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

fn main() {
    tracing_subscriber::fmt::init();

    tauri::Builder::default()
        // 動いているときにもう一度開いたら、ポートを取り合わずにウィンドウを出す
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_window(app);
        }))
        // 作り直しても大きさと位置を保つ。全画面は戻さない
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 設定は操作パネルから変えて保存する。API キーは OS の資格情報ストア
            let data_dir = app.path().app_data_dir()?;
            let settings = settings::load(&data_dir.join(settings::FILE));
            let store = Store::new(settings, data_dir);
            let idle = store.clone();
            #[cfg(not(target_os = "linux"))]
            {
                app.manage(store.clone());
                tray::install(app, store.clone())?;
            }
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

            front_window(app.handle())?;

            // 開発中は確かめない（リリースの版と比べても意味がない）
            if !cfg!(debug_assertions) {
                let app = app.handle().clone();
                // 常駐すると起動し直さないので、そのあとは字幕を止めている間に時々確かめる。
                // 断られたら、次に起動するまで聞かない
                tauri::async_runtime::spawn(async move {
                    let mut starting = true;
                    loop {
                        if starting || !idle.settings().transcription_enabled {
                            match update(app.clone()).await {
                                Ok(Answer::Declined) => return,
                                Ok(Answer::Latest) => {}
                                Err(e) => tracing::warn!("update: {e}"),
                            }
                        }
                        starting = false;
                        tokio::time::sleep(UPDATE_INTERVAL).await;
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(not(target_os = "linux"))]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                tray::close(window);
            }
            #[cfg(target_os = "linux")]
            let _ = (window, event);
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| match event {
            // ウィンドウを閉じても常駐する。終了はトレイの「終了」（code がある）だけ
            #[cfg(not(target_os = "linux"))]
            tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } => api.prevent_exit(),
            // 終了しても Chrome を残さない（残ると、次の起動で同じプロファイルを開けない）
            tauri::RunEvent::Exit => _app.state::<std::sync::Arc<Store>>().close_chrome(),
            // Mac は Dock のアイコンを押したときに、閉じたウィンドウを開き直す
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_window(_app),
            _ => {}
        });
}

fn open_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(
        app,
        "main",
        control_url().map_err(tauri::Error::InvalidUrl)?,
    )
    .title("デドロ字幕")
    // Windows ではファイルを落とす処理があるとページの中のドラッグが効かない。
    // 「OBS にドラッグして追加」を使うので切る（このアプリはファイルを受け取らない）
    .disable_drag_drop_handler()
    .inner_size(960.0, 720.0)
    // 大きさと位置を戻してから見せる
    .visible(false)
    // 外部へのリンク（API キーの作成ページなど）はふだんのブラウザで開く
    .on_new_window(|url, _| {
        if let Err(e) = tauri_plugin_opener::open_url(url.as_str(), None::<&str>) {
            tracing::warn!("cannot open {url}: {e}");
        }
        NewWindowResponse::Deny
    })
    .build()
}

/// 閉じていれば開き直す。Windows はイベントハンドラーの中でウィンドウを作ると止まるので、別のスレッドで行う
fn show_window(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = front_window(&app) {
            tracing::warn!("show window: {e}");
        }
    });
}

fn front_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let window = match app.get_webview_window("main") {
        Some(window) => window,
        None => open_window(app)?,
    };
    window.show()?;
    window.unminimize()?;
    window.set_focus()?;
    Ok(window)
}

enum Answer {
    Latest,
    Declined,
}

/// 新しい版があれば聞いて、入れ替えて再起動する（DESIGN.md 13.5）。
/// 更新情報は GitHub の最新のリリースの latest.json で、署名を公開鍵（tauri.conf.json）で確かめる
async fn update(app: AppHandle) -> Result<Answer, Box<dyn std::error::Error>> {
    let Some(update) = app.updater()?.check().await? else {
        return Ok(Answer::Latest);
    };
    tracing::info!("update available: {}", update.version);
    // 操作パネルの前に出す（親がないと、Windows ではウィンドウの裏に隠れる）
    let handle = app.clone();
    let window = tauri::async_runtime::spawn_blocking(move || front_window(&handle)).await??;
    let dialog = app
        .dialog()
        .message(format!(
            "新しい版（{}）があります。今の版は {} です。\n更新して再起動しますか？（字幕は一度止まります）",
            update.version, update.current_version
        ))
        .title("デドロ字幕の更新")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "更新する".into(),
            "あとで".into(),
        ))
        .parent(&window);
    // 答えを待つ間、非同期のワーカーを塞がない
    if !tauri::async_runtime::spawn_blocking(move || dialog.blocking_show()).await? {
        return Ok(Answer::Declined);
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
