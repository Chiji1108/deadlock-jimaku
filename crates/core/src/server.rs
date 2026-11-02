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
        .route("/assets/{version}/icons/{*path}", get(icon))
        .route("/assets/rank/{file}", get(rank_image))
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

/// URL に clientVersion を含むので、中身は変わらない（長くキャッシュしてよい）
async fn icon(
    State(store): State<Arc<Store>>,
    Path((version, path)): Path<(u32, String)>,
) -> Response {
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
    match tokio::fs::read(loaded.icons_dir(version).join(&path)).await {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (
                [
                    (header::CONTENT_TYPE, mime.to_string()),
                    (
                        header::CACHE_CONTROL,
                        "public, max-age=31536000, immutable".to_owned(),
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// 今のランクのアイコン。URL にバッジを含むので中身は変わらない
async fn rank_image(State(store): State<Arc<Store>>, Path(file): Path<String>) -> Response {
    let Some((rank, image)) = store.rank() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if file != format!("{}.webp", rank.badge) || image.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "image/webp"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        image.as_ref().clone(),
    )
        .into_response()
}

async fn ws(upgrade: WebSocketUpgrade, State(store): State<Arc<Store>>) -> Response {
    upgrade.on_upgrade(move |socket| session(socket, store))
}

/// 最初のメッセージで購読するトピックを受け取り、現在の状態を送った後は更新を流し続ける
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
    if send_all(&mut socket, &topics, store.current_state())
        .await
        .is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            msg = rx.recv() => {
                let msgs = match msg {
                    Ok(msg) => vec![msg],
                    // 取りこぼしたら、すべてのトピックの現在の状態を送り直す
                    Err(RecvError::Lagged(_)) => store.current_state(),
                    Err(RecvError::Closed) => return,
                };
                if send_all(&mut socket, &topics, msgs).await.is_err() {
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

/// 購読しているトピックのものだけを送る
async fn send_all(
    socket: &mut WebSocket,
    topics: &[Topic],
    msgs: Vec<ServerMsg>,
) -> Result<(), axum::Error> {
    for msg in msgs.iter().filter(|m| topics.contains(&m.topic())) {
        let json = serde_json::to_string(msg).expect("ServerMsg is always serializable");
        socket.send(Message::Text(json.into())).await?;
    }
    Ok(())
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
