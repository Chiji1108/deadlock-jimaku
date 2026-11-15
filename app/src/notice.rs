//! 字幕がエラーで止まったら、OS の通知で知らせる（DESIGN.md 13.6）。
//! 配信の間はゲームが前にあって、操作パネルもトレイも見ていない。

use std::sync::Arc;

use overlays_core::store::Store;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;

pub fn install(app: AppHandle, store: Arc<Store>) {
    let mut messages = store.subscribe();
    let mut failed = store.transcription_error().is_some();
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(RecvError::Closed) = messages.recv().await {
                return;
            }
            let next = store.transcription_error().is_some();
            if next == failed {
                continue;
            }
            failed = next;
            // 操作パネルを見ているなら、画面に理由が出ている
            let watching = app
                .get_webview_window("main")
                .is_some_and(|window| window.is_focused().unwrap_or(false));
            if !failed || watching {
                continue;
            }
            let shown = app
                .notification()
                .builder()
                .title("字幕が止まりました")
                .body("デドロ字幕を開いて、理由を確かめてください")
                .show();
            if let Err(e) = shown {
                tracing::warn!("notification: {e}");
            }
        }
    });
}
