//! axum: オーバーレイと操作パネルの配信、WS、REST（DESIGN.md 1, 10 章）。

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use rust_embed::RustEmbed;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::RecvError;

use crate::secrets::{self, Provider};
use crate::settings::SettingsPatch;
use crate::store::Store;
use crate::types::{ClientMsg, LanguageOption, ServerMsg, SettingsView, Topic};
use crate::{audio, lang};

#[derive(RustEmbed)]
#[folder = "../../frontend/dist"]
#[allow_missing = true]
struct Frontend;

pub fn router(store: Arc<Store>) -> Router {
    let api = Router::new()
        .route("/demo", post(demo))
        .route("/glossary", get(glossary))
        .route("/settings", get(get_settings).patch(patch_settings))
        .route("/keys/{provider}", put(put_key))
        .route("/devices", get(devices))
        .route("/languages", get(languages))
        .layer(middleware::from_fn(local_only));
    Router::new()
        .route("/ws", get(ws))
        .nest("/api", api)
        .route("/assets/{version}/icons/{*path}", get(icon))
        .route("/assets/rank/{file}", get(rank_image))
        .fallback(frontend)
        .with_state(store)
}

/// このマシンの画面（操作パネル、OBS のドック、開発中の Vite）からの呼び出しだけを受け付ける。
/// 別のサイトを開いたブラウザから設定や API キーを書き換えられないようにする（DNS リバインディング対策）。
/// 書き換えは JSON だけを受け付けるので、別のサイトからの単純なフォーム送信も通らない
async fn local_only(headers: HeaderMap, request: Request, next: Next) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let hostname = host.rsplit_once(':').map_or(host, |(name, _)| name);
    if matches!(hostname, "127.0.0.1" | "localhost" | "[::1]") {
        next.run(request).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

async fn get_settings(State(store): State<Arc<Store>>) -> Json<SettingsView> {
    let keys = tokio::task::spawn_blocking(secrets::status)
        .await
        .expect("keyring task");
    Json(SettingsView {
        settings: store.settings(),
        keys,
        unsaved: store.unsaved(),
    })
}

async fn patch_settings(
    State(store): State<Arc<Store>>,
    Json(patch): Json<SettingsPatch>,
) -> Response {
    match store.update_settings(patch) {
        Ok(_) => get_settings(State(store)).await.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct KeyBody {
    /// 空文字列なら消す
    key: String,
}

async fn put_key(
    State(store): State<Arc<Store>>,
    Path(provider): Path<Provider>,
    Json(body): Json<KeyBody>,
) -> Response {
    let saved = tokio::task::spawn_blocking(move || secrets::set(provider, &body.key))
        .await
        .expect("keyring task");
    match saved {
        Ok(()) => {
            store.restart();
            get_settings(State(store)).await.into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    }
}

async fn devices() -> Response {
    match tokio::task::spawn_blocking(audio::input_devices).await {
        Ok(Ok(names)) => Json(names).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn languages() -> Json<Vec<LanguageOption>> {
    Json(
        lang::LANGUAGES
            .iter()
            .map(|l| LanguageOption {
                code: l.code.to_owned(),
                label: l.label.to_owned(),
            })
            .collect(),
    )
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
            immutable(mime.as_ref(), bytes)
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// ランクのアイコン。URL にバッジを含み、そのバッジのアイコンだけを返すので中身は変わらない
async fn rank_image(State(store): State<Arc<Store>>, Path(file): Path<String>) -> Response {
    let image = file
        .strip_suffix(".webp")
        .and_then(|badge| badge.parse().ok())
        .and_then(|badge| store.rank_image(badge));
    match image {
        Some(bytes) => immutable("image/webp", bytes),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// URL に版やバッジを含むアセット（中身が変わらない）を長くキャッシュさせて返す
fn immutable(mime: &str, body: impl IntoResponse) -> Response {
    (
        [
            (header::CONTENT_TYPE, mime.to_owned()),
            (
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_owned(),
            ),
        ],
        body,
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
