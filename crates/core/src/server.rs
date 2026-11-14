//! axum: オーバーレイと操作パネルの配信、WS、REST（DESIGN.md 1, 10 章）。

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::RecvError;

use crate::fonts;
use crate::relay;
use crate::secrets::{self, Provider};
use crate::settings::SettingsPatch;
use crate::store::{ChromeEvent, Store};
use crate::types::{
    ChromeCommand, ChromeMsg, ClientMsg, LangCode, RankState, ServerMsg, SettingsView, Topic,
};
use crate::{audio, lang, local, people, rank};

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
        .route("/beta", post(join_beta))
        .route("/devices", get(devices))
        .route("/gpus", get(gpus))
        // 人の辞書のアイコン。画像そのものを受け取る口と、Steam やチャンネルのページから取ってくる口
        .route(
            "/people/icon",
            post(put_person_icon).layer(DefaultBodyLimit::max(5 * 1024 * 1024)),
        )
        .route("/people/icon/fetch", post(fetch_person_icon))
        // 人の辞書を、人に渡すファイルに書き出す・読み込む
        .route("/people/export", post(export_people))
        .route(
            "/people/import",
            post(import_people).layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
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
        .route("/people/{file}", get(person_icon))
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
        settings: store.saved(),
        running: store.settings().engines,
        fallback: store.fallback(),
        continues_with: store.continues_with(),
        played: store.played(),
        keys,
        unsaved: store.unsaved(),
    })
}

async fn patch_settings(
    State(store): State<Arc<Store>>,
    Json(patch): Json<SettingsPatch>,
) -> Response {
    let people = patch.people.is_some();
    match store.update_settings(patch) {
        Ok(_) => {
            // 人を消したり、アイコンを差し替えたりしたら、使わなくなった画像を消す
            if people {
                people::prune_icons(store.data_dir(), &store.saved().people).await;
            }
            get_settings(State(store)).await.into_response()
        }
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
    get_settings(State(store)).await.into_response()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(f).await.expect("blocking task")
}

// ---- ベータのコード（DESIGN.md 17.2） ----

/// ベータのコードをもらって保存する（DESIGN.md 17.2）。もう持っていれば、もらわない
async fn join_beta(State(store): State<Arc<Store>>) -> Response {
    // 続けて押されても、もらうのは 1 つ（持っているかを見てから保存するまでを、1 つずつ通す）
    static JOINING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _joining = JOINING.lock().await;
    let api = store.settings().api_base;
    if let Some(code) = blocking(secrets::beta).await {
        return Json(relay::code_status(&api, &code).await).into_response();
    }
    let code = match relay::join_beta(&api).await {
        Ok(code) => code,
        Err(refused) => return (StatusCode::BAD_REQUEST, Json(refused)).into_response(),
    };
    let saved = code.clone();
    if let Err(e) = blocking(move || secrets::set_beta(&saved)).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response();
    }
    store.restart();
    Json(relay::code_status(&api, &code).await).into_response()
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
                    Ok(ChromeMsg::Decided(decided)) => {
                        store.send_chrome_event(ChromeEvent::Decided(decided));
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

/// PC の中の AI に使える GPU の一覧。数える部品（speech.cpp）をまだ取っていなければ null
/// （一覧を見るだけのためには取らない。PC の中の AI を一度動かすと、出るようになる）
async fn gpus(State(store): State<Arc<Store>>) -> Json<Option<local::Gpus>> {
    let listed = local::gpus_if_installed(&store.data_dir().join("speech")).await;
    Json(listed.map(local::Gpus::new))
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
/// 人の辞書のアイコン。ファイルの名前は中身から作ってあるので、変わらないものとして返す
async fn person_icon(State(store): State<Arc<Store>>, Path(file): Path<String>) -> Response {
    let Some(path) = people::icon_path(store.data_dir(), &file) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&file).first_or_octet_stream();
            immutable(mime.as_ref(), bytes)
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// 操作パネルで選んだ画像を、人の辞書のアイコンとして保存する。本文は画像そのもの
async fn put_person_icon(State(store): State<Arc<Store>>, body: axum::body::Bytes) -> Response {
    match people::save_icon(store.data_dir(), &body).await {
        Ok(icon) => Json(people::Fetched { icon, name: None }).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IconSource {
    /// Twitch か YouTube のチャンネルの URL
    url: Option<String>,
    /// ランクの表示につないである Steam のアカウントから取る
    #[serde(default)]
    steam: bool,
}

/// アイコンを、Steam のプロフィールか、チャンネルのページから取ってきて保存する
async fn fetch_person_icon(
    State(store): State<Arc<Store>>,
    Json(source): Json<IconSource>,
) -> Response {
    let dir = store.data_dir();
    let fetched = match (source.url, source.steam) {
        (Some(url), _) => people::fetch_from_page(dir, &url).await,
        (None, true) => match store.saved().rank_account {
            Some(account) => people::fetch_from_steam(dir, account).await,
            None => Err(anyhow::anyhow!("no Steam account is linked")),
        },
        (None, false) => Err(anyhow::anyhow!("nothing to fetch from")),
    };
    match fetched {
        Ok(fetched) => Json(fetched).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("{e:#}")).into_response(),
    }
}

#[derive(Serialize)]
struct ExportedTo {
    /// 書き出したファイルの場所
    path: String,
}

/// 人の辞書を、ダウンロードのフォルダのファイルに書き出す（操作パネルは、アプリの中でも OBS のドックでも開くので、
/// 保存の画面は出さずに、決まった場所に置く）
async fn export_people(State(store): State<Arc<Store>>) -> Response {
    let written = async {
        let file = people::export(store.data_dir(), &store.saved().people).await?;
        let dir = dirs::download_dir().ok_or_else(|| anyhow::anyhow!("no Downloads folder"))?;
        let path = dir.join(people::EXPORT_FILE);
        tokio::fs::write(&path, file).await?;
        anyhow::Ok(path)
    };
    match written.await {
        Ok(path) => Json(ExportedTo {
            path: path.display().to_string(),
        })
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    }
}

/// 書き出したファイルの中身を読み込んで、人の辞書に足す。本文はファイルの中身
async fn import_people(State(store): State<Arc<Store>>, body: String) -> Response {
    let merged = people::import(store.data_dir(), &body, &store.saved().people).await;
    let saved = merged.map_err(|e| format!("{e:#}")).and_then(|people| {
        store.update_settings(SettingsPatch {
            people: Some(people),
            ..SettingsPatch::default()
        })
    });
    match saved {
        Ok(_) => get_settings(State(store)).await.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

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
        let list: Vec<serde_json::Value> = res.json().await.unwrap();
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
