//! axum: オーバーレイと操作パネルの配信、WS、REST（DESIGN.md 1, 10 章）。

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::RecvError;

use crate::fonts;
use crate::secrets::{self, Provider};
use crate::settings::SettingsPatch;
use crate::share::{self, GiftStatus, Refused, SharedCode};
use crate::store::{ChromeEvent, Store};
use crate::types::{
    ChromeCommand, ChromeMsg, ClientMsg, LangCode, RankState, ServerMsg, SettingsView, Topic,
};
use crate::{audio, lang, rank};
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
        .route(
            "/share/keys/{provider}",
            post(share_key).delete(unshare_key),
        )
        .route("/share/codes", post(create_code))
        .route("/share/codes/{id}", patch(update_code))
        .route("/devices", get(devices))
        .route("/languages", get(languages))
        .route("/fonts", get(fonts))
        .route("/fonts/{family}/faces.css", get(font_faces))
        .route("/fonts/{family}/download", post(download_font))
        .route("/fonts/{family}/{postscript}", get(font_file))
        .route("/rank/sample", get(rank_sample))
        .route("/chrome/open", post(open_chrome))
        .route("/chrome/reset", post(reset_chrome))
        .route("/chrome/ws", get(chrome_ws));
    Router::new()
        .route("/ws", get(ws))
        .nest("/api", api)
        .layer(middleware::from_fn(local_only))
        .route("/assets/{version}/icons/{*path}", get(icon))
        .route("/assets/rank/{file}", get(rank_image))
        .fallback(frontend)
        .with_state(store)
}

/// このマシンの画面（操作パネル、OBS のドックとオーバーレイ、開発中の Vite）からの呼び出しだけを受け付ける。
/// 別のサイトを開いたブラウザから、字幕や使用量を読んだり、設定や API キーを書き換えたりできないようにする。
/// Host は DNS リバインディング対策。Origin は別のサイトからの呼び出しの対策で、本文を読まない POST や
/// WebSocket はプリフライトなしで届くので、ブラウザが付ける Origin をここで見る。
/// Origin は呼ばれたアドレスと同じものだけを通す（Vite のプロキシは Host も Origin も `localhost:5173` のまま
/// 渡すので、ポートの値を持たなくても通る）。`Origin: null` も通さない。
/// no-cors の GET（`<img src>` など）には Origin が付かないので、`Sec-Fetch-Site: cross-site` も断る
/// （`same-site` は `localhost` の別のポートの開発の画面があるので Origin の判定に任せる）。
/// どちらも付かないものは curl などで、ブラウザの別サイトからの呼び出しではない
async fn local_only(headers: HeaderMap, request: Request, next: Next) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let hostname = host.rsplit_once(':').map_or(host, |(name, _)| name);
    let local = matches!(hostname, "127.0.0.1" | "localhost" | "[::1]");
    let same_origin = headers.get(header::ORIGIN).is_none_or(|origin| {
        origin.to_str().ok().and_then(|o| o.strip_prefix("http://")) == Some(host)
    });
    let cross_site = headers
        .get("sec-fetch-site")
        .is_some_and(|site| site == "cross-site");
    if local && same_origin && !cross_site {
        next.run(request).await
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

async fn get_settings(State(store): State<Arc<Store>>) -> Json<SettingsView> {
    let keys = tokio::task::spawn_blocking(secrets::status)
        .await
        .expect("blocking task");
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
    // その会社のキーを預けていれば、預けたキーも新しいキーにする（消したときは預けたまま。やめれば消える）。
    // 預けているか分からなければ替えない（預けていないかもしれないキーの失敗を出さない）
    if let (false, Some(token)) = (key.is_empty(), blocking(secrets::share_token).await) {
        let api = store.settings().api_base;
        let replaced = match share::shared(&api, &token).await {
            Ok(shared) if shared.providers.contains(&provider) => {
                share::set_key(&api, &token, provider, &key).await.map(Some)
            }
            Ok(_) => Ok(None),
            Err(refused) => {
                tracing::warn!("share: {}", refused.message);
                Ok(None)
            }
        };
        match replaced {
            Ok(Some(token)) => {
                if let Err(e) = blocking(move || secrets::set_share_token(&token)).await {
                    return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response();
                }
            }
            Ok(None) => {}
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
    tokio::task::spawn_blocking(f).await.expect("blocking task")
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

/// 有効なコードだけ保存する（今日の上限に達していても、明日には使える）
async fn put_gift(State(store): State<Arc<Store>>, Json(body): Json<GiftBody>) -> Response {
    let code = body.code.trim().to_owned();
    let status = share::gift_status(&store.settings().api_base, &code).await;
    if !status.is_valid() {
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
    /// キーを預けている会社（ギフトコードはどれもこの会社に使える）
    pub providers: Vec<Provider>,
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
            providers: Vec::new(),
        })
        .into_response();
    };
    match share::shared(&store.settings().api_base, &token).await {
        Ok(shared) => {
            let codes = blocking(move || {
                shared
                    .codes
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
                providers: shared.providers,
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
                providers: Vec::new(),
            })
            .into_response()
        }
        Err(refused) => refused.into_response(),
    }
}

/// ほかの PC で始めたおすそ分けを、この PC でも見る。この PC のキーのどれかを預けていれば、その人になる
async fn enable_share(State(store): State<Arc<Store>>) -> Response {
    match join_any(&store.settings().api_base).await {
        Ok(Some(token)) => save_token(store, token).await,
        Ok(None) => {
            let message = "この PC のキーで預けたおすそ分けはありません";
            (StatusCode::BAD_REQUEST, message).into_response()
        }
        Err(refused) => refused.into_response(),
    }
}

/// この PC のキーを OpenAI、xAI の順に試し、預けてあるキーの人の合言葉を返す。どれも預けていなければ None。
/// 使えないキー（期限切れなど）は飛ばして次を試す
async fn join_any(api: &str) -> Result<Option<String>, Refused> {
    for provider in [Provider::OpenAi, Provider::Xai] {
        let Some(key) = blocking(move || secrets::key(provider)).await else {
            continue;
        };
        match share::join(api, provider, &key).await {
            Ok(token) => return Ok(Some(token)),
            Err(refused) if refused.status.is_client_error() => {}
            Err(refused) => return Err(refused),
        }
    }
    Ok(None)
}

async fn save_token(store: Arc<Store>, token: String) -> Response {
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
        if let Ok(shared) = share::shared(&api, &token).await {
            ids = shared.codes.into_iter().map(|c| c.id).collect();
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

/// この PC のその会社のキーを預ける。おすそ分けしていなければ、ほかの PC で始めたおすそ分け（この PC のほかのキーを
/// 預けてあるもの）に足し、それもなければ新しく始める（同じ人が別々のおすそ分けにならないように）
async fn share_key(State(store): State<Arc<Store>>, Path(provider): Path<Provider>) -> Response {
    let Some(key) = blocking(move || secrets::key(provider)).await else {
        let message = "先に「API キー」でそのキーを保存してください";
        return (StatusCode::BAD_REQUEST, message).into_response();
    };
    let api = store.settings().api_base;
    let token = match blocking(secrets::share_token).await {
        Some(token) => Ok(Some(token)),
        None => join_any(&api).await,
    };
    let token = match token {
        Ok(Some(token)) => share::set_key(&api, &token, provider, &key).await,
        Ok(None) => share::enable(&api, provider, &key).await,
        Err(refused) => Err(refused),
    };
    match token {
        Ok(token) => save_token(store, token).await,
        Err(refused) => refused.into_response(),
    }
}

/// 預けたその会社のキーを消す。ギフトコードはほかの会社にだけ使える
async fn unshare_key(State(store): State<Arc<Store>>, Path(provider): Path<Provider>) -> Response {
    let token = match share_token().await {
        Ok(token) => token,
        Err(res) => return res.into_response(),
    };
    match share::remove_key(&store.settings().api_base, &token, provider).await {
        Ok(token) => save_token(store, token).await,
        Err(refused) => refused.into_response(),
    }
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

// ---- Chrome のモード（DESIGN.md 9.6） ----

/// 新しいページに聞く役を渡したので閉じる。ページはこの値で、つなぎ直さないと分かる
const REPLACED: u16 = 4001;

/// Chrome を開き直す
async fn open_chrome(State(store): State<Arc<Store>>) -> StatusCode {
    store.reopen_chrome();
    StatusCode::NO_CONTENT
}

/// 専用のプロファイルを消して、Chrome を始め直す
async fn reset_chrome(State(store): State<Arc<Store>>) -> Response {
    match blocking(move || store.reset_chrome()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Chrome のデータを消せません（{e}）。少し待ってから、もう一度押してください"),
        )
            .into_response(),
    }
}

async fn chrome_ws(upgrade: WebSocketUpgrade, State(store): State<Arc<Store>>) -> Response {
    upgrade.on_upgrade(move |socket| chrome_session(socket, store))
}

/// ページと文をやりとりする。聞くのは新しくつないだページだけで、古いページは閉じる
/// （オリジンの違う 127.0.0.1 と localhost の 2 つのページでも、字幕が二重に出ない）
async fn chrome_session(mut socket: WebSocket, store: Arc<Store>) {
    let (id, mut tabs) = store.connect_chrome();
    let mut commands = store.subscribe_chrome_commands();
    // ページの番号を、今の区切りの番号に揃える
    let count = store.chrome_cuts();
    let hello = serde_json::to_string(&ChromeCommand::Cut { count }).expect("chrome command");
    if socket.send(Message::text(hello)).await.is_err() {
        store.chrome_tab_closed(id);
        return;
    }
    loop {
        tokio::select! {
            biased;
            _ = async { tabs.wait_for(|tab| tab.id != id).await.map(|_| ()) } => {
                let frame = CloseFrame { code: REPLACED, reason: "replaced".into() };
                let _ = socket.send(Message::Close(Some(frame))).await;
                return;
            }
            command = commands.recv() => match command {
                Ok(command) => {
                    let text = serde_json::to_string(&command).expect("chrome command");
                    if socket.send(Message::text(text)).await.is_err() {
                        break;
                    }
                }
                // 届かなかった確定の文の訳を待ち続けない
                Err(RecvError::Lagged(_)) => store.send_chrome_event(ChromeEvent::Closed),
                Err(RecvError::Closed) => break,
            },
            message = socket.recv() => {
                let Some(Ok(message)) = message else { break };
                let Message::Text(text) = message else { continue };
                match serde_json::from_str(text.as_str()) {
                    Ok(ChromeMsg::Text(heard)) => {
                        store.send_chrome_event(ChromeEvent::Heard(heard));
                    }
                    Ok(ChromeMsg::Translated(translated)) => {
                        store.send_chrome_event(ChromeEvent::Translated(translated));
                    }
                    Ok(ChromeMsg::Status { status }) => store.set_chrome_status(id, status),
                    Err(e) => tracing::warn!("invalid chrome message: {e}"),
                }
            }
        }
    }
    store.chrome_tab_closed(id);
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

async fn fonts(State(store): State<Arc<Store>>) -> Json<Vec<fonts::FontEntry>> {
    Json(store.fonts().list().await)
}

/// カタログの名前だけを受け付ける。別の URL やパスは渡せない
async fn download_font(State(store): State<Arc<Store>>, Path(family): Path<String>) -> Response {
    let Some(font) = fonts::find(&family) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match store.fonts().download(font).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(fonts::DownloadError::Busy) => {
            (StatusCode::CONFLICT, "ダウンロード中です").into_response()
        }
        Err(fonts::DownloadError::Failed(e)) => {
            tracing::warn!("font download failed: {e:#}");
            (
                StatusCode::BAD_GATEWAY,
                "フォントをダウンロードできませんでした。時間をおいて試してください",
            )
                .into_response()
        }
    }
}

/// URL に版を含めず、毎回確かめさせる。フォントを入れ替えても同じ URL で新しい面が届く
const FONT_CACHE: (header::HeaderName, &str) = (header::CACHE_CONTROL, "no-cache");

/// ブラウザは受け取った `ETag` をそのまま返すので、完全な一致だけを見る。外れても本体を送るだけ
fn font_response(
    request: &HeaderMap,
    etag: String,
    mime: &'static str,
    body: impl IntoResponse,
) -> Response {
    let unchanged = request
        .get(header::IF_NONE_MATCH)
        .is_some_and(|sent| sent == etag.as_str());
    let etag = [(header::ETAG, etag)];
    if unchanged {
        return (StatusCode::NOT_MODIFIED, [FONT_CACHE], etag).into_response();
    }
    ([(header::CONTENT_TYPE, mime), FONT_CACHE], etag, body).into_response()
}

async fn font_faces(
    State(store): State<Arc<Store>>,
    Path(family): Path<String>,
    headers: HeaderMap,
) -> Response {
    match store.fonts().faces_css(family).await {
        Some(css) => font_response(
            &headers,
            fonts::content_etag(&css),
            "text/css; charset=utf-8",
            css,
        ),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn font_file(
    State(store): State<Arc<Store>>,
    Path((family, postscript)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    match store.fonts().face_file(family, postscript).await {
        Some(face) => font_response(&headers, face.etag, "font/sfnt", face.data),
        None => StatusCode::NOT_FOUND.into_response(),
    }
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

/// ランクのアイコン。名前に元の画像の URL のハッシュを含むので、同じ URL の中身は変わらない
async fn rank_image(State(store): State<Arc<Store>>, Path(file): Path<String>) -> Response {
    let Some(path) = rank::image_path(&store.rank_images(), &file) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(path).await {
        Ok(bytes) => immutable("image/webp", bytes),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct RankSampleQuery {
    lang: LangCode,
}

/// プレビューの見本のランク。`lang` の名前とアイコンは、まだ取れていなければ取ってくる。取れなかったものは欠けたまま
async fn rank_sample(
    State(store): State<Arc<Store>>,
    Query(query): Query<RankSampleQuery>,
) -> Result<Json<RankState>, StatusCode> {
    let language = lang::find(&query.lang).ok_or(StatusCode::BAD_REQUEST)?;
    Ok(Json(rank::sample(&store, language).await))
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
            // ページは更新されるので保存させない（中のスクリプトは名前が変わる）
            let headers = [
                (header::CONTENT_TYPE, mime.as_ref()),
                (header::CACHE_CONTROL, "no-cache"),
            ];
            return (headers, file.data).into_response();
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::path::PathBuf;

    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

    use super::*;
    use crate::settings::Settings;
    use crate::types::{Heard, LiveState, LiveStatus};

    /// 手元のポートで動かす。アドレス（`127.0.0.1:…`）を返す
    async fn serve_on_any_port() -> String {
        serve_with_data(PathBuf::new()).await
    }

    async fn serve_with_data(data_dir: PathBuf) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let store = Store::new(Ok(Settings::default()), data_dir);
        tokio::spawn(serve(listener, store));
        addr
    }

    #[tokio::test]
    async fn serves_the_fonts_as_a_list() {
        let addr = serve_on_any_port().await;
        let res = reqwest::get(format!("http://{addr}/api/fonts"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // PC のフォントは PC によるので、一覧の作り方は fonts.rs のテストで確かめる
        let list: Vec<Value> = res.json().await.unwrap();
        assert!(list.iter().any(|font| font["family"] == "Zen Maru Gothic"));
    }

    #[tokio::test]
    async fn only_catalog_fonts_can_be_downloaded() {
        let addr = serve_on_any_port().await;
        for family in ["Arial", "No%20Such%20Font", "..%2F..%2Fetc%2Fpasswd"] {
            let res = reqwest::Client::new()
                .post(format!("http://{addr}/api/fonts/{family}/download"))
                .send()
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{family}");
        }
    }

    fn font_request(sent: &str) -> Response {
        let mut request = HeaderMap::new();
        request.insert(header::IF_NONE_MATCH, sent.parse().unwrap());
        font_response(&request, r#""v2""#.to_owned(), "font/sfnt", "body")
    }

    #[test]
    fn a_font_the_browser_has_is_not_sent_again() {
        let res = font_request(r#""v2""#);
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(res.headers()[header::ETAG], r#""v2""#);
        assert_eq!(res.headers()[header::CACHE_CONTROL], "no-cache");
    }

    #[tokio::test]
    async fn a_changed_font_is_sent_with_its_new_etag() {
        let res = font_request(r#""v1""#);
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::ETAG], r#""v2""#);
        assert_eq!(res.headers()[header::CACHE_CONTROL], "no-cache");
        assert_eq!(res.headers()[header::CONTENT_TYPE], "font/sfnt");
        let body = axum::body::to_bytes(res.into_body(), 16).await.unwrap();
        assert_eq!(&body[..], b"body");
    }

    #[tokio::test]
    async fn serves_only_the_saved_rank_icons() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("rank")).unwrap();
        std::fs::write(dir.path().join("rank/73-0123456789abcdef.webp"), b"webp").unwrap();
        std::fs::write(dir.path().join("rank/73.webp"), b"webp").unwrap();
        let addr = serve_with_data(dir.path().to_owned()).await;
        let res = reqwest::get(format!(
            "http://{addr}/assets/rank/73-0123456789abcdef.webp"
        ))
        .await
        .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "image/webp");
        assert_eq!(&res.bytes().await.unwrap()[..], b"webp");
        for file in [
            "73.webp",
            "74-0123456789abcdef.webp",
            "73-0123456789abcdef.png",
            "..%2Frank%2F73-0123456789abcdef.webp",
        ] {
            let res = reqwest::get(format!("http://{addr}/assets/rank/{file}"))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{file}");
        }
    }

    #[tokio::test]
    async fn the_rank_sample_needs_a_known_language() {
        let addr = serve_on_any_port().await;
        for query in ["", "?lang=xx"] {
            let res = reqwest::get(format!("http://{addr}/api/rank/sample{query}"))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{query}");
        }
    }

    #[tokio::test]
    async fn font_urls_of_unknown_fonts_are_not_found() {
        let addr = serve_on_any_port().await;
        for path in [
            "No%20Such%20Font/faces.css",
            "No%20Such%20Font/Regular",
            "..%2F..%2F..%2Fetc/passwd",
            "%2Fetc%2Fpasswd/faces.css",
            "No%20Such%20Font/..%2F..%2F..%2Fetc%2Fpasswd",
            "No%20Such%20Font/C:%5CWindows%5Cwin.ini",
        ] {
            let res = reqwest::get(format!("http://{addr}/api/fonts/{path}"))
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn fonts_are_not_served_to_other_sites() {
        let addr = serve_on_any_port().await;
        let res = reqwest::Client::new()
            .get(format!("http://{addr}/api/fonts/Arial/faces.css"))
            .header("sec-fetch-site", "cross-site")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    async fn post_reset(addr: &str) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("http://{addr}/api/chrome/reset"))
            .send()
            .await
            .unwrap()
    }

    async fn post_demo(addr: &str, headers: &[(&str, &str)]) -> StatusCode {
        let mut req = reqwest::Client::new().post(format!("http://{addr}/api/demo"));
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        req.send().await.unwrap().status()
    }

    /// 秘密を読まない GET。呼び出しの入り口の判定だけを見る
    async fn get_settings_status(addr: &str, headers: &[(&str, &str)]) -> StatusCode {
        let mut req = reqwest::Client::new().get(format!("http://{addr}/api/languages"));
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        req.send().await.unwrap().status()
    }

    /// `/ws` につなぐ。断られたらその状態、つながったら 101
    async fn ws_status(addr: &str, headers: &[(&'static str, &str)]) -> StatusCode {
        let mut req = format!("ws://{addr}/ws").into_client_request().unwrap();
        for (name, value) in headers {
            req.headers_mut().insert(*name, value.parse().unwrap());
        }
        match tokio_tungstenite::connect_async(req).await {
            Ok((_, res)) => res.status(),
            Err(tungstenite::Error::Http(res)) => res.status(),
            Err(e) => panic!("{e}"),
        }
    }

    #[tokio::test]
    async fn other_sites_cannot_reach_the_api_or_the_websocket() {
        let addr = serve_on_any_port().await;
        let port = addr.rsplit_once(':').unwrap().1;
        for origin in [
            "https://evil.example",
            "null",
            // 同じマシンの別のポート、https、呼ばれたのと違う名前
            "http://127.0.0.1:1",
            &format!("https://{addr}"),
            &format!("http://localhost:{port}"),
        ] {
            let origin = [("origin", origin)];
            assert_eq!(
                post_demo(&addr, &origin).await,
                StatusCode::FORBIDDEN,
                "{origin:?}"
            );
            assert_eq!(
                ws_status(&addr, &origin).await,
                StatusCode::FORBIDDEN,
                "{origin:?}"
            );
        }
        // no-cors の GET には Origin が付かない
        let cross_site = [("sec-fetch-site", "cross-site")];
        assert_eq!(
            get_settings_status(&addr, &cross_site).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(post_demo(&addr, &cross_site).await, StatusCode::FORBIDDEN);
        assert_eq!(ws_status(&addr, &cross_site).await, StatusCode::FORBIDDEN);
        // DNS リバインディング（Host が別の名前）は Origin が揃っていても通らない
        let rebinding = [
            ("host", "evil.example:4510"),
            ("origin", "http://evil.example:4510"),
        ];
        assert_eq!(post_demo(&addr, &rebinding).await, StatusCode::FORBIDDEN);
        assert_eq!(ws_status(&addr, &rebinding).await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn other_sites_cannot_reach_the_chrome_page_routes() {
        let addr = serve_on_any_port().await;
        let client = reqwest::Client::new();
        let open = client
            .post(format!("http://{addr}/api/chrome/open"))
            .header("origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(open.status(), StatusCode::FORBIDDEN);
        let reset = client
            .post(format!("http://{addr}/api/chrome/reset"))
            .header("origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(reset.status(), StatusCode::FORBIDDEN);
        let mut req = format!("ws://{addr}/api/chrome/ws")
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("origin", "https://evil.example".parse().unwrap());
        let refused = tokio_tungstenite::connect_async(req).await;
        assert!(
            matches!(refused, Err(tungstenite::Error::Http(res)) if res.status() == StatusCode::FORBIDDEN)
        );
    }

    #[tokio::test]
    async fn reset_removes_only_the_chrome_profile_and_can_be_repeated() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("chrome-profile");
        std::fs::create_dir_all(profile.join("Default")).unwrap();
        std::fs::write(profile.join("Default/Preferences"), "{}").unwrap();
        let settings = dir.path().join("settings.json");
        std::fs::write(&settings, "{}").unwrap();
        let addr = serve_with_data(dir.path().to_owned()).await;

        assert_eq!(post_reset(&addr).await.status(), StatusCode::NO_CONTENT);
        assert!(!profile.exists());
        assert!(settings.exists());
        assert_eq!(post_reset(&addr).await.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn reset_tells_the_user_when_the_profile_cannot_be_removed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("chrome-profile"), "").unwrap();
        let addr = serve_with_data(dir.path().to_owned()).await;

        let response = post_reset(&addr).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let message = response.text().await.unwrap();
        assert!(message.contains("Chrome のデータを消せません"), "{message}");
    }

    #[tokio::test]
    async fn the_chrome_page_sends_text_and_status_until_it_closes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let store = Store::new(Ok(Settings::default()), PathBuf::new());
        tokio::spawn(serve(listener, store.clone()));
        let mut events = store.subscribe_chrome();
        let mut messages = store.subscribe();
        let (mut socket, _) = page(addr).await;

        let status = r#"{ "type": "status", "status": { "state": "listening" } }"#;
        socket.send(status.into()).await.unwrap();
        let text = r#"{ "type": "text", "text": "ヘイズ", "final": false, "cut": 0 }"#;
        socket.send(text.into()).await.unwrap();
        assert_eq!(
            messages.recv().await.unwrap(),
            ServerMsg::ChromeStatus {
                status: Some(LiveStatus {
                    state: LiveState::Listening,
                    detail: None,
                }),
            }
        );
        assert_eq!(
            events.recv().await.unwrap(),
            ChromeEvent::Heard(Heard {
                text: "ヘイズ".into(),
                is_final: false,
                cut: 0,
            })
        );

        socket.close(None).await.unwrap();
        assert_eq!(
            events.recv().await.unwrap(),
            ChromeEvent::Closed,
            "開いている発話を捨てる"
        );
        assert_eq!(
            messages.recv().await.unwrap(),
            ServerMsg::ChromeStatus { status: None }
        );
    }

    async fn page(
        addr: SocketAddr,
    ) -> (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ) {
        tokio_tungstenite::connect_async(format!("ws://{addr}/api/chrome/ws"))
            .await
            .unwrap()
    }

    async fn next_status(messages: &mut tokio::sync::broadcast::Receiver<ServerMsg>) -> LiveState {
        loop {
            if let ServerMsg::ChromeStatus { status: Some(s) } = messages.recv().await.unwrap() {
                return s.state;
            }
        }
    }

    #[tokio::test]
    async fn a_new_page_replaces_the_old_one_and_keeps_its_own_state() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let store = Store::new(Ok(Settings::default()), PathBuf::new());
        tokio::spawn(serve(listener, store.clone()));
        let mut messages = store.subscribe();

        let (mut old, _) = page(addr).await;
        let listening = r#"{ "type": "status", "status": { "state": "listening" } }"#;
        old.send(listening.into()).await.unwrap();
        assert_eq!(next_status(&mut messages).await, LiveState::Listening);

        let (mut new, _) = page(addr).await;
        let closed = loop {
            match old.next().await.unwrap().unwrap() {
                tungstenite::Message::Close(frame) => break frame.unwrap(),
                _ => continue,
            }
        };
        assert_eq!(u16::from(closed.code), REPLACED);
        drop(old);

        let preparing = r#"{ "type": "status", "status": { "state": "preparing" } }"#;
        new.send(preparing.into()).await.unwrap();
        assert_eq!(next_status(&mut messages).await, LiveState::Preparing);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let snapshot = store.snapshot_chrome_status();
        assert_eq!(snapshot.map(|s| s.state), Some(LiveState::Preparing));
    }

    #[tokio::test]
    async fn the_app_itself_and_callers_without_origin_get_through() {
        let addr = serve_on_any_port().await;
        let port = addr.rsplit_once(':').unwrap().1;
        let localhost = format!("localhost:{port}");
        let ipv6 = format!("[::1]:{port}");
        for (host, origin) in [
            (addr.as_str(), format!("http://{addr}")),
            (localhost.as_str(), format!("http://{localhost}")),
            (ipv6.as_str(), format!("http://{ipv6}")),
            // Vite のプロキシ（`frontend/vite.config.ts`）が渡す形
            ("localhost:5173", "http://localhost:5173".to_owned()),
        ] {
            let headers = [("host", host), ("origin", origin.as_str())];
            assert_eq!(
                post_demo(&addr, &headers).await,
                StatusCode::ACCEPTED,
                "{host}"
            );
            assert_eq!(
                ws_status(&addr, &headers).await,
                StatusCode::SWITCHING_PROTOCOLS,
                "{host}"
            );
        }
        assert_eq!(post_demo(&addr, &[]).await, StatusCode::ACCEPTED);
        assert_eq!(ws_status(&addr, &[]).await, StatusCode::SWITCHING_PROTOCOLS);
        // 操作パネルやオーバーレイ自身の呼び出しと、URL を開いたとき。same-site は Origin の判定に任せる
        for site in ["same-origin", "none", "same-site"] {
            let headers = [("sec-fetch-site", site)];
            assert_eq!(post_demo(&addr, &headers).await, StatusCode::ACCEPTED);
            assert_eq!(
                ws_status(&addr, &headers).await,
                StatusCode::SWITCHING_PROTOCOLS
            );
        }
    }
}
