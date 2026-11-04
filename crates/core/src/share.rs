//! おすそ分けとギフトコード（DESIGN.md 17.1）。開発者のサーバー（Worker）を呼ぶ。
//! おすそ分けの合言葉はここで資格情報ストアから読んで付け、操作パネルには渡さない

use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

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
    },
    Expired {
        #[ts(type = "number")]
        expires_at: i64,
    },
    /// おすそ分けした人が止めた
    Revoked,
    /// ないコード（打ち間違い、おすそ分けをやめた）
    Unknown,
    /// サーバーにつながらない
    Unreachable { detail: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GiftAnswer {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    expires_at: Option<i64>,
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
        (StatusCode::OK, _, expires_at) => GiftStatus::Active { expires_at },
        (_, Some("expired"), Some(expires_at)) => GiftStatus::Expired { expires_at },
        (_, Some("revoked"), _) => GiftStatus::Revoked,
        (StatusCode::UNAUTHORIZED, _, _) => GiftStatus::Unknown,
        _ => GiftStatus::Unreachable {
            detail: status.to_string(),
        },
    }
}

/// 字幕を始めるときに、ギフトコードがまだ使えるか確かめる
pub async fn check_gift(api: &str, code: &str) -> Result<(), TranscriptionError> {
    match gift_status(api, code).await {
        GiftStatus::Active { .. } => Ok(()),
        GiftStatus::Expired { expires_at } => Err(TranscriptionError::GiftExpired { expires_at }),
        GiftStatus::Revoked => Err(TranscriptionError::GiftRevoked),
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

/// サーバーを呼ぶ。断られたら日本語の理由にする。`token` は管理の合言葉（有効にするときはまだない）
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
        Some("invalidKey" | "noKey") => "OpenAI の API キーが使えません".to_owned(),
        Some("keyInUse") => "そのキーでは、別におすそ分けを有効にしています".to_owned(),
        Some("unauthorized") => {
            "おすそ分けの登録が見つかりません。無効にしてから、もう一度有効にしてください"
                .to_owned()
        }
        Some("notFound") => "コードが見つかりません".to_owned(),
        _ => format!("サーバーのエラー（{status}）"),
    };
    Err(Refused { status, message })
}

/// おすそ分けを有効にする（キーを預ける）。管理の合言葉を返す
pub async fn enable(api: &str, key: &str) -> Result<String, Refused> {
    let body = json!({ "key": key });
    let value = call(api, reqwest::Method::POST, "/share", None, Some(body)).await?;
    value["token"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Refused {
            status: StatusCode::BAD_GATEWAY,
            message: "サーバーの答えを読めません".to_owned(),
        })
}

pub async fn disable(api: &str, token: &str) -> Result<(), Refused> {
    call(api, reqwest::Method::DELETE, "/share", Some(token), None)
        .await
        .map(drop)
}

pub async fn update_key(api: &str, token: &str, key: &str) -> Result<(), Refused> {
    let body = json!({ "key": key });
    call(
        api,
        reqwest::Method::PUT,
        "/share/key",
        Some(token),
        Some(body),
    )
    .await
    .map(drop)
}

pub async fn codes(api: &str, token: &str) -> Result<Vec<SharedCode>, Refused> {
    let value = call(api, reqwest::Method::GET, "/share/codes", Some(token), None).await?;
    serde_json::from_value(value["codes"].clone()).map_err(|e| Refused {
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

/// メモ・期限・止めるかを変える（`body` はそのままサーバーへ）
pub async fn update(api: &str, token: &str, id: &str, body: Value) -> Result<(), Refused> {
    let path = format!("/share/codes/{id}");
    call(api, reqwest::Method::PATCH, &path, Some(token), Some(body))
        .await
        .map(drop)
}
