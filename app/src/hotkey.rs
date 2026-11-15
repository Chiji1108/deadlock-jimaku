//! どのアプリが前にあっても、決めたキーで字幕を始める・止める（設定の `toggle_hotkey`。DESIGN.md 13.6）。
//! 配信の間はゲームが前にあるので、押した結果は OS の通知で返す。

use std::sync::Arc;

use overlays_core::store::Store;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tokio::sync::broadcast::error::RecvError;

use crate::{notice, toggle_subtitles};

pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _, event| {
            if event.state() == ShortcutState::Pressed {
                let store = app.state::<Arc<Store>>().inner().clone();
                toggle_subtitles(app.clone(), store, true);
            }
        })
        .build()
}

/// 設定のキーを登録し、設定が変わったら登録し直す
pub fn install(app: AppHandle, store: Arc<Store>) {
    let mut messages = store.subscribe();
    let mut key = store.settings().toggle_hotkey;
    register(&app, key.as_deref());
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(RecvError::Closed) = messages.recv().await {
                return;
            }
            let next = store.settings().toggle_hotkey;
            if next != key {
                key = next;
                register(&app, key.as_deref());
            }
        }
    });
}

/// このアプリが登録するキーは 1 つだけなので、全部外してから登録する
fn register(app: &AppHandle, key: Option<&str>) {
    let shortcuts = app.global_shortcut();
    if let Err(e) = shortcuts.unregister_all() {
        tracing::warn!("hotkey: {e}");
    }
    let Some(key) = key else { return };
    if let Err(e) = shortcuts.register(key) {
        tracing::warn!("hotkey {key}: {e}");
        notice::show(
            app,
            "キーを登録できませんでした",
            "ほかのアプリが使っているキーかもしれません。デドロ字幕で、別のキーを選んでください",
        );
    }
}

#[cfg(test)]
mod tests {
    use tauri_plugin_global_shortcut::Shortcut;

    /// 操作パネルが作る形（`HotkeyCard.svelte`。修飾キー + `KeyboardEvent.code`）を、OS に登録する形に読める
    #[test]
    fn keys_from_the_panel_are_read() {
        for key in [
            "Ctrl+Shift+F9",
            "Ctrl+Alt+Shift+Super+KeyJ",
            "Alt+Digit1",
            "F13",
        ] {
            assert!(key.parse::<Shortcut>().is_ok(), "{key}");
        }
    }
}
