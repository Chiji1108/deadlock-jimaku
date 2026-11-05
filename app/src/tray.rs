//! ウィンドウを閉じても字幕とオーバーレイを止めないよう、トレイに常駐する（DESIGN.md 13.6）。
//! Linux はトレイの出ない環境があり、隠すと戻せなくなるので使わない。

use std::sync::Arc;

use overlays_core::settings::SettingsPatch;
use overlays_core::store::Store;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use crate::show_window;

const OPEN: &str = "open";
const TOGGLE: &str = "toggle";
const QUIT: &str = "quit";

#[cfg(target_os = "macos")]
const NOTICE: &str = "デドロ字幕は閉じても動き続けます。終了するときは、メニューバーのアイコンから「終了」を選んでください。";
#[cfg(not(target_os = "macos"))]
const NOTICE: &str = "デドロ字幕は閉じても動き続けます。終了するときは、タスクバーの通知領域のアイコンから「終了」を選んでください。";

fn toggle_label(enabled: bool) -> &'static str {
    if enabled {
        "字幕を止める"
    } else {
        "字幕を始める"
    }
}

fn tooltip(enabled: bool) -> &'static str {
    if enabled {
        "デドロ字幕（字幕を出しています）"
    } else {
        "デドロ字幕（停止中）"
    }
}

pub fn install(app: &App, store: Arc<Store>) -> tauri::Result<()> {
    let enabled = store.settings().transcription_enabled;
    let toggle = MenuItem::with_id(app, TOGGLE, toggle_label(enabled), true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, OPEN, "開く", true, None::<&str>)?,
            &toggle,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, QUIT, "終了", true, None::<&str>)?,
        ],
    )?;
    let on_menu = store.clone();
    let tray = TrayIconBuilder::new()
        .icon(tauri::include_image!("icons/32x32.png"))
        .tooltip(tooltip(enabled))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            OPEN => show_window(app),
            TOGGLE => toggle_subtitles(&on_menu),
            QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;

    // 操作パネルで変えても、メニューと説明を今の状態にする
    let mut settings = store.watch_settings();
    tauri::async_runtime::spawn(async move {
        while settings.changed().await.is_ok() {
            let enabled = settings.borrow_and_update().transcription_enabled;
            if let Err(e) = toggle.set_text(toggle_label(enabled)) {
                tracing::warn!("tray menu: {e}");
            }
            if let Err(e) = tray.set_tooltip(Some(tooltip(enabled))) {
                tracing::warn!("tray tooltip: {e}");
            }
        }
    });
    Ok(())
}

fn toggle_subtitles(store: &Store) {
    let patch = SettingsPatch {
        transcription_enabled: Some(!store.settings().transcription_enabled),
        ..Default::default()
    };
    if let Err(e) = store.update_settings(patch) {
        tracing::warn!("tray toggle: {e}");
    }
}

/// 閉じるボタンではウィンドウを隠すだけにする。動き続けていることは初回だけ知らせる
pub fn hide(window: &WebviewWindow) {
    if let Err(e) = window.hide() {
        tracing::warn!("hide window: {e}");
    }
    let app = window.app_handle();
    let store = app.state::<Arc<Store>>();
    if store.settings().tray_notice_seen {
        return;
    }
    notify(app);
    let patch = SettingsPatch {
        tray_notice_seen: Some(true),
        ..Default::default()
    };
    if let Err(e) = store.update_settings(patch) {
        tracing::warn!("tray notice: {e}");
    }
}

fn notify(app: &AppHandle) {
    app.dialog()
        .message(NOTICE)
        .title("デドロ字幕")
        .show(|_| {});
}
