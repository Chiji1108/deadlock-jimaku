//! axum: オーバーレイと操作パネルの配信、WS、REST（DESIGN.md 1, 10 章）。

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::RecvError;

use crate::secrets::{self, Provider};
use crate::settings::SettingsPatch;
use crate::share::{self, GiftStatus, Refused, SharedCode};
use crate::store::Store;
use crate::types::{ClientMsg, LangCode, Segment, ServerMsg, SettingsView, Topic};
use crate::{audio, lang};
use ts_rs::TS;

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
        .route("/gift", get(get_gift).put(put_gift).delete(delete_gift))
        .route(
            "/share",
            get(get_share).post(enable_share).delete(disable_share),
        )
        .route("/share/codes", post(create_code))
        .route("/share/codes/{id}", patch(update_code))
        .route("/devices", get(devices))
        .route("/languages", get(languages))
        .route("/segments", post(post_segment))
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

/// Chrome のページが認識した字幕を受け取る（DESIGN.md 9.6）
async fn post_segment(State(store): State<Arc<Store>>, Json(segment): Json<Segment>) -> StatusCode {
    store.upsert_segment(segment);
    StatusCode::NO_CONTENT
}

async fn get_settings(State(store): State<Arc<Store>>) -> Json<SettingsView> {
    let keys = tokio::task::spawn_blocking(secrets::status)
        .await
        .expect("keyring task");
    Json(SettingsView {
        version: env!("CARGO_PKG_VERSION"),
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
    let key = body.key.trim().to_owned();
    let saved = {
        let key = key.clone();
        blocking(move || secrets::set(provider, &key)).await
    };
    if let Err(e) = saved {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response();
    }
    store.restart();
    // おすそ分けしていれば、預けたキーも新しいキーにする（消したときは預けたまま。無効にすれば消える）
    if let (Provider::OpenAi, false, Some(token)) = (
        provider,
        key.is_empty(),
        blocking(secrets::share_token).await,
    ) {
        match share::update_key(&store.settings().api_base, &token, &key).await {
            Ok(token) => {
                if let Err(e) = blocking(move || secrets::set_share_token(&token)).await {
                    return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response();
                }
            }
            // ほかの PC でやめた、キーを替えたなど。預けたキーはもうない
            Err(refused) if refused.status == StatusCode::UNAUTHORIZED => {}
            Err(refused) => {
                let message = format!(
                    "キーは保存しましたが、おすそ分けに預けたキーを替えられませんでした（{}）",
                    refused.message
                );
                return (StatusCode::BAD_GATEWAY, message).into_response();
            }
        }
    }
    get_settings(State(store)).await.into_response()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("keyring task")
}

// ---- ギフトコード（人からもらったもの。DESIGN.md 17.1） ----

async fn get_gift(State(store): State<Arc<Store>>) -> Json<GiftStatus> {
    Json(match blocking(secrets::gift).await {
        Some(code) => share::gift_status(&store.settings().api_base, &code).await,
        None => GiftStatus::None,
    })
}

#[derive(Deserialize)]
struct GiftBody {
    code: String,
}

/// 使えるコードだけ保存する
async fn put_gift(State(store): State<Arc<Store>>, Json(body): Json<GiftBody>) -> Response {
    let code = body.code.trim().to_owned();
    let status = share::gift_status(&store.settings().api_base, &code).await;
    if !matches!(status, GiftStatus::Active { .. }) {
        return (StatusCode::BAD_REQUEST, Json(status)).into_response();
    }
    if let Err(e) = blocking(move || secrets::set_gift(&code)).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response();
    }
    store.restart();
    Json(status).into_response()
}

async fn delete_gift(State(store): State<Arc<Store>>) -> Response {
    if let Err(e) = blocking(|| secrets::set_gift("")).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response();
    }
    store.restart();
    StatusCode::NO_CONTENT.into_response()
}

// ---- おすそ分け（自分のキーで人に使ってもらう） ----

#[derive(Serialize, TS)]
#[ts(export)]
pub struct ShareView {
    pub enabled: bool,
    pub codes: Vec<SharedCode>,
}

impl IntoResponse for Refused {
    fn into_response(self) -> Response {
        let status = if self.status.is_server_error() {
            StatusCode::BAD_GATEWAY
        } else {
            StatusCode::BAD_REQUEST
        };
        (status, self.message).into_response()
    }
}

async fn get_share(State(store): State<Arc<Store>>) -> Response {
    let Some(token) = blocking(secrets::share_token).await else {
        return Json(ShareView {
            enabled: false,
            codes: Vec::new(),
        })
        .into_response();
    };
    match share::codes(&store.settings().api_base, &token).await {
        Ok(codes) => {
            let codes = blocking(move || {
                codes
                    .into_iter()
                    .map(|c| SharedCode {
                        code: secrets::share_code(&c.id),
                        ..c
                    })
                    .collect()
            })
            .await;
            Json(ShareView {
                enabled: true,
                codes,
            })
            .into_response()
        }
        // ほかの PC でやめた、キーを替えたなど。この PC は預ける前の状態に戻す
        Err(refused) if refused.status == StatusCode::UNAUTHORIZED => {
            if let Err(e) = blocking(|| secrets::set_share_token("")).await {
                return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response();
            }
            Json(ShareView {
                enabled: false,
                codes: Vec::new(),
            })
            .into_response()
        }
        Err(refused) => refused.into_response(),
    }
}

/// 自分のキーを預ける（初めてギフトコードを作るとき）
async fn enable_share(State(store): State<Arc<Store>>) -> Response {
    let Some(key) = blocking(secrets::openai).await else {
        let message = "先に自分の OpenAI の API キーを保存してください";
        return (StatusCode::BAD_REQUEST, message).into_response();
    };
    let token = match share::enable(&store.settings().api_base, &key).await {
        Ok(token) => token,
        Err(refused) => return refused.into_response(),
    };
    if let Err(e) = blocking(move || secrets::set_share_token(&token)).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response();
    }
    get_share(State(store)).await
}

/// 預けたキーとコードを消す。サーバーに登録が残っていなくても、こちらの合言葉とコードは消す
async fn disable_share(State(store): State<Arc<Store>>) -> Response {
    let api = store.settings().api_base;
    let mut ids = Vec::new();
    if let Some(token) = blocking(secrets::share_token).await {
        // 消す前に、この PC に覚えたコードの ID を聞いておく
        if let Ok(codes) = share::codes(&api, &token).await {
            ids = codes.into_iter().map(|c| c.id).collect();
        }
        if let Err(refused) = share::disable(&api, &token).await
            && refused.status != StatusCode::UNAUTHORIZED
        {
            return refused.into_response();
        }
    }
    let cleared = blocking(move || {
        for id in &ids {
            secrets::set_share_code(id, "")?;
        }
        secrets::set_share_token("")
    })
    .await;
    if let Err(e) = cleared {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response();
    }
    get_share(State(store)).await
}

async fn share_token() -> Result<String, (StatusCode, &'static str)> {
    blocking(secrets::share_token)
        .await
        .ok_or((StatusCode::BAD_REQUEST, "おすそ分けを使っていません"))
}

/// コードを作り、この PC に覚えておく（あとでコピーできるように）
async fn create_code(State(store): State<Arc<Store>>, Json(body): Json<Value>) -> Response {
    let token = match share_token().await {
        Ok(token) => token,
        Err(res) => return res.into_response(),
    };
    let (id, code) = match share::create(&store.settings().api_base, &token, body).await {
        Ok(created) => created,
        Err(refused) => return refused.into_response(),
    };
    let saved = {
        let code = code.clone();
        blocking(move || secrets::set_share_code(&id, &code)).await
    };
    // サーバーはハッシュしか持たないので、ここで覚えられなければ二度と出せない。今だけ見せる
    if let Err(e) = saved {
        tracing::warn!("share code: {e:#}");
        let message = format!(
            "コードは作りましたが、この PC に保存できませんでした。今のうちに控えてください: {code}"
        );
        return (StatusCode::INTERNAL_SERVER_ERROR, message).into_response();
    }
    get_share(State(store)).await
}

/// メモ・期限を変える、止める・戻す
async fn update_code(
    State(store): State<Arc<Store>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let token = match share_token().await {
        Ok(token) => token,
        Err(res) => return res.into_response(),
    };
    if let Err(refused) = share::update(&store.settings().api_base, &token, &id, body).await {
        return refused.into_response();
    }
    get_share(State(store)).await
}

async fn devices() -> Response {
    match tokio::task::spawn_blocking(audio::input_devices).await {
        Ok(Ok(names)) => Json(names).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// 対応している言語のコード。表示名は画面の言語に合わせて操作パネルが作る
async fn languages() -> Json<Vec<LangCode>> {
    Json(lang::LANGUAGES.iter().map(|l| l.code.to_owned()).collect())
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
    let wants_level = topics.contains(&Topic::Level);
    let mut level = store.watch_level();
    let wants_usage = topics.contains(&Topic::Usage);
    let mut usage = store.meter().watch();
    usage.mark_unchanged();
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
            Ok(()) = level.changed(), if wants_level => {
                let db = *level.borrow_and_update();
                if send_all(&mut socket, &topics, vec![ServerMsg::AudioLevel { db }]).await.is_err() {
                    return;
                }
            }
            Ok(()) = usage.changed(), if wants_usage => {
                let usage = usage.borrow_and_update().clone();
                if send_all(&mut socket, &topics, vec![ServerMsg::Usage { usage }]).await.is_err() {
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
