// リリースビルドで Windows のコンソールウィンドウを出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod hotkey;
mod notice;
#[cfg(not(target_os = "linux"))]
mod tray;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use overlays_core::settings::SettingsPatch;
use overlays_core::{port, runtime, secrets, server, settings, store::Store};
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
        .plugin(tauri_plugin_notification::init())
        .plugin(hotkey::plugin())
        .setup(|app| {
            // 設定は操作パネルから変えて保存する。API キーは OS の資格情報ストア
            let data_dir = match overlays_core::data_dir_override() {
                Some(dir) => dir,
                None => app.path().app_data_dir()?,
            };
            let settings = settings::load(&data_dir.join(settings::FILE));
            let store = Store::new(settings, data_dir);
            let idle = store.clone();
            app.manage(store.clone());
            #[cfg(not(target_os = "linux"))]
            tray::install(app, store.clone())?;
            notice::install(app.handle().clone(), store.clone());
            hotkey::install(app.handle().clone(), store.clone());
            tauri::async_runtime::spawn(runtime::run(
                store.clone(),
                runtime::Options { input_file: None },
            ));
            // ウィンドウを開く前に bind して、ポートが使えないことを起動時に検出する
            let listener = tauri::async_runtime::block_on(TcpListener::bind(SocketAddr::from((
                Ipv4Addr::LOCALHOST,
                port(),
            ))))?;
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server::serve(listener, store).await {
                    tracing::error!("server stopped: {e}");
                }
            });

            front_window(app.handle())?;

            // 開発中は確かめない（リリースの版と比べても意味がない）
            if !cfg!(debug_assertions) {
                tauri::async_runtime::spawn(watch_updates(app.handle().clone(), idle));
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
            // 終了しても Chrome を残さない（残ると、次の起動で同じプロファイルを開けない）。
            // PC の中の文字起こしと LLM のプロセスも残さない
            tauri::RunEvent::Exit => {
                _app.state::<Arc<Store>>().close_chrome();
                overlays_core::local::stop();
                overlays_core::llama::stop();
                overlays_core::whisper::stop();
            }
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

/// 字幕を始める・止める（トレイのメニューとキー）。キーがないと字幕は始められないので、始める代わりに操作パネルを開く。
/// `notify` なら、結果を OS の通知で返す（キーで押したときは、画面を見ていない）
fn toggle_subtitles(app: AppHandle, store: Arc<Store>, notify: bool) {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = store.settings();
        let start = !settings.transcription_enabled;
        // 字幕の処理が始めるときに確かめるのと同じ決まり（工程が使う会社ごとに、自分のキーか、中継するコード）
        let keys = secrets::load();
        let mut needed = settings.engines.providers().into_iter();
        if start && needed.any(|p| keys.access(p, &settings.api_base).is_none()) {
            show_window(&app);
            return;
        }
        let patch = SettingsPatch {
            transcription_enabled: Some(start),
            ..Default::default()
        };
        match store.update_settings(patch) {
            Ok(_) if notify && start => notice::show(&app, "字幕を始めました", ""),
            Ok(_) if notify => notice::show(&app, "字幕を止めました", ""),
            Ok(_) => {}
            Err(e) => tracing::warn!("toggle subtitles: {e}"),
        }
    });
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

/// 新しい版があるかを、起動したときと、そのあと時々確かめる（DESIGN.md 13.5）。あれば操作パネルに出し続ける
/// （`Store::set_update`。字幕を出している間も確かめる）。常駐すると起動し直さないので、ダイアログでは
/// 起動したときか、字幕を止めている間に 1 度だけ聞く。断られたら、次に起動するまで聞かない
/// （操作パネルの「更新する」は残る）。
/// 更新情報は GitHub の最新のリリースの latest.json で、署名を公開鍵（tauri.conf.json）で確かめる
async fn watch_updates(app: AppHandle, store: Arc<Store>) {
    let mut ask = true;
    let mut starting = true;
    loop {
        // 操作パネルの「更新する」は、もう聞かずに入れ替える
        let requested = tokio::select! {
            () = store.update_requested() => true,
            () = tokio::time::sleep(match starting {
                true => Duration::ZERO,
                false => UPDATE_INTERVAL,
            }) => false,
        };
        let idle = starting || !store.settings().transcription_enabled;
        starting = false;
        let update = match app.updater() {
            Ok(updater) => updater.check().await,
            Err(e) => Err(e),
        };
        let update = match update {
            Ok(update) => update,
            Err(e) => {
                tracing::warn!("update: {e}");
                if requested {
                    failed(&app, &e.to_string());
                }
                continue;
            }
        };
        store.set_update(update.as_ref().map(|update| update.version.clone()));
        let Some(update) = update else {
            continue;
        };
        tracing::info!("update available: {}", update.version);
        if !requested {
            if !ask || !idle {
                continue;
            }
            match confirmed(&app, &update).await {
                Ok(true) => {}
                Ok(false) => {
                    ask = false;
                    continue;
                }
                Err(e) => {
                    tracing::warn!("update: {e}");
                    continue;
                }
            }
        }
        match update.download_and_install(|_, _| {}, || {}).await {
            Ok(()) => app.restart(),
            Err(e) => {
                tracing::warn!("update: {e}");
                failed(&app, &e.to_string());
            }
        }
    }
}

/// 「更新しますか？」と聞く。「更新する」なら true
async fn confirmed(
    app: &AppHandle,
    update: &tauri_plugin_updater::Update,
) -> Result<bool, Box<dyn std::error::Error>> {
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
    Ok(tauri::async_runtime::spawn_blocking(move || dialog.blocking_show()).await?)
}

/// 更新できなかったことを知らせる（操作パネルの「更新する」を押した人は、結果を待っている）
fn failed(app: &AppHandle, detail: &str) {
    app.dialog()
        .message(format!(
            "更新できませんでした。少し待って、もう一度試してください。\n{detail}"
        ))
        .title("デドロ字幕の更新")
        .show(|_| {});
}

/// 操作パネルは OBS のドックと同じく localhost の画面を開く（IPC は使わない）。
/// 開発中は Vite（devUrl）から開き、/api と /ws は Vite が axum にプロキシする。
fn control_url() -> Result<WebviewUrl, url::ParseError> {
    if cfg!(debug_assertions) {
        Ok(WebviewUrl::App("control.html".into()))
    } else {
        Ok(WebviewUrl::External(
            format!("http://127.0.0.1:{}/", port()).parse()?,
        ))
    }
}
