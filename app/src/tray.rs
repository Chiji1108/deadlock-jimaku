//! ウィンドウを閉じても字幕とオーバーレイを止めないよう、トレイに常駐する（DESIGN.md 13.6）。
//! Linux はトレイの出ない環境があり、閉じると戻せなくなるので使わない。

use std::sync::Arc;

use overlays_core::secrets;
use overlays_core::settings::{SettingsPatch, SpeechEngine};
use overlays_core::store::Store;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, Window};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::broadcast::error::RecvError;

use crate::show_window;

const OPEN: &str = "open";
const TOGGLE: &str = "toggle";
const QUIT: &str = "quit";

#[cfg(target_os = "macos")]
const NOTICE: &str = "デドロ字幕は閉じても動き続けます。終了するときは、メニューバーのアイコンから「終了」を選んでください。";
#[cfg(not(target_os = "macos"))]
const NOTICE: &str = "デドロ字幕は閉じても動き続けます。終了するときは、タスクバーの通知領域のアイコンから「終了」を選んでください。";

/// トレイに出す状態。字幕が止まったことは、ウィンドウを閉じていてもここで分かるようにする
#[derive(Clone, Copy, PartialEq)]
enum State {
    Stopped,
    Running,
    Failed,
}

impl State {
    fn of(store: &Store) -> Self {
        match (
            store.settings().transcription_enabled,
            store.transcription_error(),
        ) {
            (false, _) => Self::Stopped,
            (true, None) => Self::Running,
            (true, Some(_)) => Self::Failed,
        }
    }

    fn toggle_label(self) -> &'static str {
        match self {
            Self::Stopped => "字幕を始める",
            Self::Running | Self::Failed => "字幕を止める",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::Stopped => "デドロ字幕（停止中）",
            Self::Running => "デドロ字幕（字幕を出しています）",
            Self::Failed => "デドロ字幕（字幕が止まっています）",
        }
    }
}

pub fn install(app: &App, store: Arc<Store>) -> tauri::Result<()> {
    let mut state = State::of(&store);
    let toggle = MenuItem::with_id(app, TOGGLE, state.toggle_label(), true, None::<&str>)?;
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
        .tooltip(state.tooltip())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            OPEN => show_window(app),
            TOGGLE => toggle_subtitles(app.clone(), on_menu.clone()),
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

    let mut messages = store.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(RecvError::Closed) = messages.recv().await {
                return;
            }
            let next = State::of(&store);
            if next == state {
                continue;
            }
            state = next;
            if let Err(e) = toggle.set_text(state.toggle_label()) {
                tracing::warn!("tray menu: {e}");
            }
            if let Err(e) = tray.set_tooltip(Some(state.tooltip())) {
                tracing::warn!("tray tooltip: {e}");
            }
        }
    });
    Ok(())
}

/// キーがないと字幕は始められないので、始める代わりに操作パネルを開く
fn toggle_subtitles(app: AppHandle, store: Arc<Store>) {
    tauri::async_runtime::spawn_blocking(move || {
        let settings = store.settings();
        let start = !settings.transcription_enabled;
        let needs_key = settings.speech_engine != SpeechEngine::Chrome;
        if start && needs_key && secrets::openai().is_none() && secrets::gift().is_none() {
            show_window(&app);
            return;
        }
        let patch = SettingsPatch {
            transcription_enabled: Some(start),
            ..Default::default()
        };
        if let Err(e) = store.update_settings(patch) {
            tracing::warn!("tray toggle: {e}");
        }
    });
}

/// 閉じるボタンで WebView ごとウィンドウを閉じ、メモリを空ける。字幕はアプリの本体で動き続ける。
/// 動き続けていることは初回だけ、閉じる前に知らせる
pub fn close(window: &Window) {
    let store = window.state::<Arc<Store>>();
    if store.settings().tray_notice_seen {
        destroy(window);
        return;
    }
    let shown = window.clone();
    window
        .dialog()
        .message(NOTICE)
        .title("デドロ字幕")
        .parent(window)
        .show(move |_| {
            let patch = SettingsPatch {
                tray_notice_seen: Some(true),
                ..Default::default()
            };
            if let Err(e) = shown.state::<Arc<Store>>().update_settings(patch) {
                tracing::warn!("tray notice: {e}");
            }
            destroy(&shown);
        });
}

fn destroy(window: &Window) {
    if let Err(e) = window.destroy() {
        tracing::warn!("close window: {e}");
    }
}
