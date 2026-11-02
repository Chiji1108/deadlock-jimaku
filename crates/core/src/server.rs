//! axum: オーバーレイと操作パネルの配信、WS、REST（DESIGN.md 1, 10 章）。

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use rust_embed::RustEmbed;
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::RecvError;

use crate::store::Store;
use crate::types::{ClientMsg, ServerMsg, Topic};

#[derive(RustEmbed)]
#[folder = "../../frontend/dist"]
#[allow_missing = true]
struct Frontend;

pub fn router(store: Arc<Store>) -> Router {
    Router::new()
        .route("/ws", get(ws))
        .route("/api/demo", post(demo))
        .route("/api/glossary", get(glossary))
        .route("/assets/icons/{*path}", get(icon))
        .fallback(frontend)
        .with_state(store)
}

pub async fn serve(listener: TcpListener, store: Arc<Store>) -> std::io::Result<()> {
    axum::serve(listener, router(store)).await
}

async fn demo(State(store): State<Arc<Store>>) -> StatusCode {
    crate::demo::spawn_once(store);
    StatusCode::ACCEPTED
}

async fn glossary(State(store): State<Arc<Store>>) -> Response {
    match store.glossary() {
        Some(loaded) => Json(loaded.glossary.as_ref()).into_response(),
        // 起動直後で辞書をまだ作っている。オーバーレイは glossary.updated を待って取り直す
        None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// 辞書と同じ clientVersion のディレクトリからアイコンを返す
async fn icon(State(store): State<Arc<Store>>, Path(path): Path<String>) -> Response {
    let safe = path.split('/').all(|seg| {
        !seg.is_empty()
            && seg != ".."
            && seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    });
    let Some(loaded) = store.glossary().filter(|_| safe) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(loaded.icons_dir().join(&path)).await {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (
                [
                    (header::CONTENT_TYPE, mime.to_string()),
                    (header::CACHE_CONTROL, "public, max-age=86400".to_owned()),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn ws(upgrade: WebSocketUpgrade, State(store): State<Arc<Store>>) -> Response {
    upgrade.on_upgrade(move |socket| session(socket, store))
}

/// 最初のメッセージで購読するトピックを受け取り、snapshot の後は更新を流し続ける
async fn session(mut socket: WebSocket, store: Arc<Store>) {
    let topics = match socket.recv().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str(text.as_str()) {
            Ok(ClientMsg::Subscribe { topics }) => topics,
            Err(e) => {
                tracing::warn!("invalid subscribe: {e}");
                return;
            }
        },
        _ => return,
    };

    let mut rx = store.subscribe();
    let snapshot = store.snapshot();
    if wants(&topics, &snapshot) && send(&mut socket, &snapshot).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            msg = rx.recv() => {
                let msg = match msg {
                    Ok(msg) => msg,
                    // 取りこぼしたら snapshot で状態ごと置き換える
                    Err(RecvError::Lagged(_)) => store.snapshot(),
                    Err(RecvError::Closed) => return,
                };
                if wants(&topics, &msg) && send(&mut socket, &msg).await.is_err() {
                    return;
                }
            }
            incoming = socket.recv() => {
                if !matches!(incoming, Some(Ok(_))) {
                    return;
                }
            }
        }
    }
}

fn wants(topics: &[Topic], msg: &ServerMsg) -> bool {
    topics.contains(&msg.topic())
}

async fn send(socket: &mut WebSocket, msg: &ServerMsg) -> Result<(), axum::Error> {
    let json = serde_json::to_string(msg).expect("ServerMsg is always serializable");
    socket.send(Message::Text(json.into())).await
}

/// `/` → control.html、`/overlay/subtitles` → overlay/subtitles.html
async fn frontend(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let candidates = if path.is_empty() {
        vec!["control.html".to_owned()]
    } else {
        vec![path.to_owned(), format!("{path}.html")]
    };
    for candidate in candidates {
        if let Some(file) = Frontend::get(&candidate) {
            let mime = mime_guess::from_path(&candidate).first_or_octet_stream();
            return ([(header::CONTENT_TYPE, mime.as_ref())], file.data).into_response();
        }
    }
    StatusCode::NOT_FOUND.into_response()
}
