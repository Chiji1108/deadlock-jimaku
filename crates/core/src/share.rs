//! おすそ分けとギフトコード（DESIGN.md 17.1）。開発者のサーバー（Worker）を呼ぶ。
//! おすそ分けの合言葉はここで資格情報ストアから読んで付け、操作パネルには渡さない

use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

use crate::access::Access;
use crate::secrets::{Provider, Relay};
use crate::types::TranscriptionError;

const TIMEOUT: Duration = Duration::from_secs(10);

/// 人からもらったギフトコードの状態
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum GiftStatus {
    /// 保存していない
    None,
    Active {
        /// ミリ秒。None は期限なし
        #[ts(type = "number | null")]
        expires_at: Option<i64>,
        /// 中継する会社（おすそ分けした人がキーを預けた会社）
        providers: Vec<Provider>,
    },
    Expired {
        #[ts(type = "number")]
        expires_at: i64,
    },
    /// おすそ分けした人が止めた
    Revoked,
    /// 今日（UTC）の上限に達した
    Limit,
    /// ないコード（打ち間違い、おすそ分けをやめた）
    Unknown,
    /// サーバーにつながらない
    Unreachable { detail: String },
}

impl GiftStatus {
    /// 持っていてよいコードか。今日の上限は日が変われば戻るので、今は使えなくても有効
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Active { .. } | Self::Limit)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GiftAnswer {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    expires_at: Option<i64>,
    /// 前のサーバーは返さない（OpenAI だけを中継する）
    #[serde(default = "openai_only")]
    providers: Vec<Provider>,
}

fn openai_only() -> Vec<Provider> {
    vec![Provider::OpenAi]
}

/// サーバーが今日の上限で断るときの理由。JSON の `error`、Realtime の close の reason、
/// [`REFUSAL_HEADER`] で同じ値を使う
pub const LIMIT: &str = "limit";

/// おすそ分けした人がその会社のキーを預けていないので、サーバーが中継を断るときの理由
pub const UNSUPPORTED: &str = "unsupported";

/// サーバーが中継を断った理由のヘッダー。WebSocket のハンドシェイクでは、断られたときの本文は
/// 届いた分しか読めないので、これで見分ける
pub const REFUSAL_HEADER: &str = "x-gift-refusal";

/// 失敗した応答の本文から、Worker が断った理由を読む。OpenAI の `error` はオブジェクトなので読まない
pub fn refusal_reason(body: &str) -> Option<String> {
    serde_json::from_str::<GiftAnswer>(body).ok()?.error
}

/// 送り直しても断られるだけの応答なら、字幕を止める理由を返す（DESIGN.md 17.1）。
/// ギフトコードの 401 は理由を読まない。ハンドシェイクでは本文を読みきれずヘッダーもなく、Worker を通った
/// OpenAI の 401（預けたキーが使えない）も、おすそ分けした人に確かめてもらうのは同じ
pub fn refusal(
    access: &Access,
    status: StatusCode,
    reason: Option<&str>,
) -> Option<TranscriptionError> {
    let refused = match (status, reason) {
        (StatusCode::UNAUTHORIZED, _) if access.is_gift() => Some(TranscriptionError::GiftUnknown),
        (StatusCode::UNAUTHORIZED, _) => Some(TranscriptionError::InvalidKey {
            provider: access.provider(),
        }),
        // 403 の理由は Worker のもの。xAI の `error` も文字列なので、自分のキーでは読まない
        (StatusCode::FORBIDDEN, _) if !access.is_gift() => None,
        (StatusCode::FORBIDDEN, Some("revoked")) => Some(TranscriptionError::GiftRevoked),
        (StatusCode::FORBIDDEN, Some("expired")) => {
            Some(TranscriptionError::GiftExpired { expires_at: None })
        }
        (StatusCode::FORBIDDEN, Some(LIMIT)) => Some(TranscriptionError::GiftLimit),
        (StatusCode::FORBIDDEN, Some(UNSUPPORTED)) => Some(TranscriptionError::GiftUnsupported {
            provider: access.provider(),
        }),
        _ => None,
    };
    refused.map(|e| for_code(e, access.provider()))
}

/// その会社を中継するのがベータのコードなら、ベータの理由にする。ギフトコードの理由は、おすそ分けした人に
/// 確かめるよう案内するので、開発者が出したコードには合わない
pub fn for_code(refused: TranscriptionError, provider: Provider) -> TranscriptionError {
    use TranscriptionError as E;
    if provider.info().relay != Some(Relay::Beta) {
        return refused;
    }
    match refused {
        E::GiftLimit => E::BetaLimit,
        E::GiftRevoked | E::GiftUnknown | E::GiftExpired { .. } => E::BetaRefused,
        other => other,
    }
}

pub async fn gift_status(api: &str, code: &str) -> GiftStatus {
    let res = crate::http()
        .get(format!("{api}/gift"))
        .bearer_auth(code)
        .timeout(TIMEOUT)
        .send()
        .await;
    let res = match res {
        Ok(res) => res,
        Err(e) => {
            return GiftStatus::Unreachable {
                detail: format!("{e:#}"),
            };
        }
    };
    let status = res.status();
    let answer: GiftAnswer = match res.json().await {
        Ok(answer) => answer,
        Err(e) => {
            return GiftStatus::Unreachable {
                detail: format!("{status}: {e:#}"),
            };
        }
    };
    match (status, answer.error.as_deref(), answer.expires_at) {
        (StatusCode::OK, _, expires_at) => GiftStatus::Active {
            expires_at,
            providers: answer.providers,
        },
        (_, Some("expired"), Some(expires_at)) => GiftStatus::Expired { expires_at },
        (_, Some("revoked"), _) => GiftStatus::Revoked,
        (_, Some("limit"), _) => GiftStatus::Limit,
        (StatusCode::UNAUTHORIZED, _, _) => GiftStatus::Unknown,
        _ => GiftStatus::Unreachable {
            detail: status.to_string(),
        },
    }
}

/// ベータのコードをもらえなかった理由
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(tag = "error", rename_all = "camelCase")]
#[ts(export)]
pub enum BetaRefusal {
    /// ベータの提供が終わった（開発者がキーを入れていない）
    Closed,
    /// 同じ所から、今日はもう何度ももらっている
    TooMany,
    /// 今日出す分が終わった
    Full,
    /// サーバーにつながらない
    Unreachable { detail: String },
}

/// ベータのコードを 1 つもらう（DESIGN.md 17.2）。ベータのモードを初めて選んだときに頼む
pub async fn join_beta(api: &str) -> Result<String, BetaRefusal> {
    let unreachable = |detail: String| BetaRefusal::Unreachable { detail };
    let res = crate::http()
        .post(format!("{api}/beta/codes"))
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| unreachable(format!("{e:#}")))?;
    #[derive(Deserialize)]
    struct Answer {
        code: Option<String>,
        error: Option<String>,
    }
    let status = res.status();
    let answer: Answer = res
        .json()
        .await
        .map_err(|e| unreachable(format!("{status}: {e:#}")))?;
    match (answer.code, answer.error.as_deref()) {
        (Some(code), _) if status.is_success() => Ok(code),
        (_, Some("closed")) => Err(BetaRefusal::Closed),
        (_, Some("tooMany")) => Err(BetaRefusal::TooMany),
        (_, Some("full")) => Err(BetaRefusal::Full),
        _ => Err(unreachable(status.to_string())),
    }
}

/// 動いていない字幕を始めるときに、ギフトコードがまだ使えるか確かめる。使えれば中継する会社
pub async fn check_gift(api: &str, code: &str) -> Result<Vec<Provider>, TranscriptionError> {
    match gift_status(api, code).await {
        GiftStatus::Active { providers, .. } => Ok(providers),
        GiftStatus::Expired { expires_at } => Err(TranscriptionError::GiftExpired {
            expires_at: Some(expires_at),
        }),
        GiftStatus::Revoked => Err(TranscriptionError::GiftRevoked),
        GiftStatus::Limit => Err(TranscriptionError::GiftLimit),
        GiftStatus::Unknown | GiftStatus::None => Err(TranscriptionError::GiftUnknown),
        GiftStatus::Unreachable { detail } => Err(TranscriptionError::Failed {
            detail: format!("gift: {detail}"),
        }),
    }
}

/// おすそ分けのコード（自分が作ったもの）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SharedCode {
    pub id: String,
    /// この PC で作ったコードだけ分かる（サーバーはハッシュしか持たない）
    #[serde(default)]
    #[ts(optional)]
    pub code: Option<String>,
    pub memo: String,
    /// ミリ秒。None は期限なし
    #[ts(type = "number | null")]
    pub expires_at: Option<i64>,
    pub revoked: bool,
    /// これまでに使われた額（USD）
    pub usd: f64,
    /// 今月（UTC）使われた額
    pub month_usd: f64,
    /// 今日（UTC）使われた額
    pub day_usd: f64,
    /// 実際に効く 1 日（UTC）の上限（USD）
    pub effective_limit_usd: f64,
    #[ts(type = "number")]
    pub created_at: i64,
}

/// サーバーの答えのうち、操作パネルに見せるもの。断られたら日本語の理由
#[derive(Debug)]
pub struct Refused {
    pub status: StatusCode,
    pub message: String,
}

impl Refused {
    fn from_error(e: reqwest::Error) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: format!("サーバーにつながりません（{e}）"),
        }
    }
}

/// サーバーを呼ぶ。断られたら日本語の理由にする。`token` は管理の合言葉（キーを預けるときはまだない）
async fn call(
    api: &str,
    method: reqwest::Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Result<Value, Refused> {
    let mut req = crate::http()
        .request(method, format!("{api}{path}"))
        .timeout(TIMEOUT);
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    if let Some(body) = body {
        req = req.json(&body);
    }
    let res = req.send().await.map_err(Refused::from_error)?;
    let status = res.status();
    let text = res.text().await.map_err(Refused::from_error)?;
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(value);
    }
    let message = match value["error"].as_str() {
        Some("invalidKey") => "OpenAI の API キーが使えません".to_owned(),
        Some("invalidXaiKey") => "xAI の API キーが使えません".to_owned(),
        Some("keyInUse") => "そのキーは、別のおすそ分けで使っています".to_owned(),
        Some("lastKey") => {
            "最後のキーは預けるのをやめられません。やめるときは「おすそ分けをやめる」を押してください"
                .to_owned()
        }
        Some("unauthorized") => {
            "おすそ分けの登録が見つかりません。「おすそ分けをやめる」を押してから、ギフトコードを作り直してください"
                .to_owned()
        }
        Some("notFound") => "コードが見つかりません".to_owned(),
        _ => format!("サーバーのエラー（{status}）"),
    };
    Err(Refused { status, message })
}

/// キーを預けておすそ分けを始める。同じキーをもう預けていれば、その人になる。管理の合言葉を返す
pub async fn enable(api: &str, provider: Provider, key: &str) -> Result<String, Refused> {
    let body = json!({ "provider": provider, "key": key });
    let value = call(api, reqwest::Method::POST, "/share", None, Some(body)).await?;
    token_of(&value)
}

/// ほかの PC で預けたキーと同じなら、その人の合言葉を返す。まだ預けていなければ 404（新しく始めない）
pub async fn join(api: &str, provider: Provider, key: &str) -> Result<String, Refused> {
    let body = json!({ "provider": provider, "key": key, "join": true });
    let value = call(api, reqwest::Method::POST, "/share", None, Some(body)).await?;
    token_of(&value)
}

pub async fn disable(api: &str, token: &str) -> Result<(), Refused> {
    call(api, reqwest::Method::DELETE, "/share", Some(token), None)
        .await
        .map(drop)
}

/// その会社のキーを預ける・替える。新しい合言葉を返す（替えたときは前の合言葉が使えなくなる）
pub async fn set_key(
    api: &str,
    token: &str,
    provider: Provider,
    key: &str,
) -> Result<String, Refused> {
    let path = format!("/share/keys/{}", provider_path(provider));
    let body = json!({ "key": key });
    let value = call(api, reqwest::Method::PUT, &path, Some(token), Some(body)).await?;
    token_of(&value)
}

/// 預けたその会社のキーを消す（最後の 1 つは消せない）。新しい合言葉を返す（前の合言葉は使えなくなる）
pub async fn remove_key(api: &str, token: &str, provider: Provider) -> Result<String, Refused> {
    let path = format!("/share/keys/{}", provider_path(provider));
    let value = call(api, reqwest::Method::DELETE, &path, Some(token), None).await?;
    token_of(&value)
}

/// パスに入れる会社の名前（serde と同じ "openai" / "xai"）
fn provider_path(provider: Provider) -> String {
    match serde_json::to_value(provider) {
        Ok(Value::String(name)) => name,
        _ => unreachable!("Provider serializes to a string"),
    }
}

fn token_of(value: &Value) -> Result<String, Refused> {
    value["token"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Refused {
            status: StatusCode::BAD_GATEWAY,
            message: "サーバーの答えを読めません".to_owned(),
        })
}

/// 預けているもの（作ったコードと、キーを預けた会社）
#[derive(Deserialize)]
pub struct Shared {
    pub codes: Vec<SharedCode>,
    /// 前のサーバーは返さない（OpenAI のキーだけを預かる）
    #[serde(default = "openai_only")]
    pub providers: Vec<Provider>,
}

pub async fn shared(api: &str, token: &str) -> Result<Shared, Refused> {
    let value = call(api, reqwest::Method::GET, "/share/codes", Some(token), None).await?;
    serde_json::from_value(value).map_err(|e| Refused {
        status: StatusCode::BAD_GATEWAY,
        message: format!("サーバーの答えを読めません（{e}）"),
    })
}

/// コードを作る。(ID, コード)
pub async fn create(api: &str, token: &str, body: Value) -> Result<(String, String), Refused> {
    let value = call(
        api,
        reqwest::Method::POST,
        "/share/codes",
        Some(token),
        Some(body),
    )
    .await?;
    match (value["id"].as_str(), value["code"].as_str()) {
        (Some(id), Some(code)) => Ok((id.to_owned(), code.to_owned())),
        _ => Err(Refused {
            status: StatusCode::BAD_GATEWAY,
            message: "サーバーの答えを読めません".to_owned(),
        }),
    }
}

/// メモ・期限・1 日の上限・止めるかを変える（`body` はそのままサーバーへ）
pub async fn update(api: &str, token: &str, id: &str, body: Value) -> Result<(), Refused> {
    let path = format!("/share/codes/{id}");
    call(api, reqwest::Method::PATCH, &path, Some(token), Some(body))
        .await
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Worker に、ベータのコードで、アプリの部品が実際に送る要求を通す。手元の Worker（`wrangler dev`。偽の開発者のキー）なら、
    /// 要求の形が Worker の検査を通れば外に送り、偽のキーなので「外の不調」（502）が返る（400 なら形が合っていない）。
    /// 本番（`https://api.jimaku.deadlock.jp/v1`）なら、文字起こしと訳が返る:
    /// `BETA_TEST_API=http://127.0.0.1:8799/v1 cargo test -p overlays-core beta_requests -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "needs a Worker"]
    async fn beta_requests_pass_the_worker() {
        use crate::enrich::Span;
        use crate::settings::Engines;
        use crate::stt::{BatchRequest, Hints, Transcriber};
        use crate::usage::Meter;
        use std::sync::Arc;

        let api = std::env::var("BETA_TEST_API").unwrap();
        let code = join_beta(&api).await.unwrap();
        let providers = check_gift(&api, &code).await.unwrap();
        assert_eq!(providers, [Provider::Qwen, Provider::OpenRouter]);
        let gift = |provider| Access::gift(provider, code.clone(), &api);
        let meter = Arc::new(Meter::new(
            std::env::temp_dir().join("overlays-beta-test.json"),
        ));

        let wav = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../experiments/data/sample01.wav"
        );
        let request = BatchRequest {
            transcriber: &Transcriber::Api(gift(Provider::Qwen)),
            model: "qwen-audio-3.1-asr-flash",
            hints: &Hints {
                language: "ja".into(),
                prompt: String::new(),
                keywords: vec!["ヘイズ".into()],
            },
            wav: std::fs::read(wav).unwrap(),
            seconds: 21.8,
            meter: &meter,
        };
        match crate::stt::transcribe(crate::http(), request).await {
            Ok(text) => println!("transcription: {text}"),
            // 偽のキーが外で断られ、Worker は「外の不調」（502）として返す
            Err(e) => assert!(format!("{e:#}").contains("upstream"), "{e:#}"),
        }

        let mt = Engines::BETA.translation.mt().unwrap();
        let translator = crate::mt::Translator::new(
            gift(mt.provider),
            mt.name,
            "ja".into(),
            vec!["en".into()],
            meter.clone(),
        );
        let spans = [Span::Text("強い".into())];
        // 翻訳の失敗は、その文の訳を諦めるだけ（エラーにならず、訳が空になる）。Worker が 502 を返したことは、Worker の記録で見る
        let translated = translator.translate(crate::http(), &spans, None).await;
        println!("translation: {:?}", translated.unwrap());
    }

    #[test]
    fn a_refused_beta_code_gets_its_own_reason() {
        use TranscriptionError as E;
        let beta = Access::gift(Provider::Qwen, "code".into(), "http://worker");
        let refused = |reason| refusal(&beta, StatusCode::FORBIDDEN, Some(reason));
        assert_eq!(refused("limit"), Some(E::BetaLimit));
        assert_eq!(refused("revoked"), Some(E::BetaRefused));
        assert_eq!(
            refusal(&beta, StatusCode::UNAUTHORIZED, None),
            Some(E::BetaRefused)
        );
        // 中継しない口は、今までどおり
        let unsupported = E::GiftUnsupported {
            provider: Provider::Qwen,
        };
        assert_eq!(refused(UNSUPPORTED), Some(unsupported));
        // 人からもらったギフトコードの理由は変えない
        assert_eq!(for_code(E::GiftLimit, Provider::OpenAi), E::GiftLimit);
    }

    #[test]
    fn a_gift_at_its_daily_limit_stays_valid() {
        assert!(GiftStatus::Limit.is_valid());
        let active = GiftStatus::Active {
            expires_at: None,
            providers: vec![Provider::OpenAi],
        };
        assert!(active.is_valid());
        assert!(!GiftStatus::Revoked.is_valid());
        assert!(!GiftStatus::Unknown.is_valid());
    }

    #[test]
    fn only_answers_that_would_repeat_stop_the_subtitles() {
        let gift = Access::gift(Provider::OpenAi, "code".into(), "http://worker");
        let key = Access::key(Provider::OpenAi, "sk".into());
        let xai_gift = Access::gift(Provider::Xai, "code".into(), "http://worker");
        let xai_key = Access::key(Provider::Xai, "xai".into());
        let refusal =
            |access, status, body| refusal(access, status, refusal_reason(body).as_deref());
        for (access, status, body, expected) in [
            (
                &gift,
                StatusCode::UNAUTHORIZED,
                r#"{"error":"unknown"}"#,
                Some(TranscriptionError::GiftUnknown),
            ),
            (
                &gift,
                StatusCode::UNAUTHORIZED,
                "",
                Some(TranscriptionError::GiftUnknown),
            ),
            (
                &gift,
                StatusCode::FORBIDDEN,
                r#"{"error":"revoked"}"#,
                Some(TranscriptionError::GiftRevoked),
            ),
            (
                &gift,
                StatusCode::FORBIDDEN,
                r#"{"error":"expired"}"#,
                Some(TranscriptionError::GiftExpired { expires_at: None }),
            ),
            (
                &gift,
                StatusCode::FORBIDDEN,
                r#"{"error":"limit"}"#,
                Some(TranscriptionError::GiftLimit),
            ),
            (
                &xai_gift,
                StatusCode::FORBIDDEN,
                r#"{"error":"unsupported"}"#,
                Some(TranscriptionError::GiftUnsupported {
                    provider: Provider::Xai,
                }),
            ),
            (
                &xai_gift,
                StatusCode::UNAUTHORIZED,
                "",
                Some(TranscriptionError::GiftUnknown),
            ),
            (
                &xai_key,
                StatusCode::FORBIDDEN,
                r#"{"error":"limit"}"#,
                None,
            ),
            (
                &xai_key,
                StatusCode::UNAUTHORIZED,
                "",
                Some(TranscriptionError::InvalidKey {
                    provider: Provider::Xai,
                }),
            ),
            (
                &key,
                StatusCode::UNAUTHORIZED,
                r#"{"error":{"code":"invalid_api_key"}}"#,
                Some(TranscriptionError::InvalidKey {
                    provider: Provider::OpenAi,
                }),
            ),
            // OpenAI の 403 の error はオブジェクト
            (
                &key,
                StatusCode::FORBIDDEN,
                r#"{"error":{"message":"no"}}"#,
                None,
            ),
            (&gift, StatusCode::BAD_REQUEST, r#"{"error":"limit"}"#, None),
            (
                &gift,
                StatusCode::TOO_MANY_REQUESTS,
                r#"{"error":"rate"}"#,
                None,
            ),
            (
                &key,
                StatusCode::TOO_MANY_REQUESTS,
                r#"{"error":{"code":"rate_limit_exceeded"}}"#,
                None,
            ),
            (&key, StatusCode::INTERNAL_SERVER_ERROR, "", None),
            (&gift, StatusCode::BAD_GATEWAY, "<html>", None),
        ] {
            assert_eq!(refusal(access, status, body), expected, "{status} {body}");
        }
    }

    #[tokio::test]
    async fn a_gift_at_its_daily_limit_does_not_start() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let refuse = || async {
            (
                axum::http::StatusCode::FORBIDDEN,
                axum::Json(json!({ "error": "limit" })),
            )
        };
        let router = axum::Router::new().route("/gift", axum::routing::get(refuse));
        tokio::spawn(async move { axum::serve(listener, router).await });
        assert_eq!(gift_status(&api, "code").await, GiftStatus::Limit);
        assert_eq!(
            check_gift(&api, "code").await,
            Err(TranscriptionError::GiftLimit)
        );
    }
}
