//! 開発者のサーバー（Worker）が中継するコード（DESIGN.md 17.2）。ベータのコードをもらう、まだ使えるか確かめる、
//! 中継に断られた理由を読む。0.7 までは、人のキーで中継するギフトコード（おすそ分け）もここで扱っていた

use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::access::Access;
use crate::secrets::Provider;
use crate::types::TranscriptionError;

const TIMEOUT: Duration = Duration::from_secs(10);

/// コードの状態
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum CodeStatus {
    /// 保存していない
    None,
    Active {
        /// 中継する会社
        providers: Vec<Provider>,
    },
    /// 止められた
    Revoked,
    /// 今日（UTC）の上限に達した
    Limit,
    /// ないコード
    Unknown,
    /// サーバーにつながらない
    Unreachable { detail: String },
}

impl CodeStatus {
    /// 持っていてよいコードか。今日の上限は日が変われば戻るので、今は使えなくても有効
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Active { .. } | Self::Limit)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodeAnswer {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    providers: Vec<Provider>,
}

/// サーバーが今日の上限で断るときの理由。JSON の `error`、Realtime の close の reason、
/// [`REFUSAL_HEADER`] で同じ値を使う
pub const LIMIT: &str = "limit";

/// そのコードでは中継しない会社なので、サーバーが中継を断るときの理由
pub const UNSUPPORTED: &str = "unsupported";

/// サーバーが中継を断った理由のヘッダー。WebSocket のハンドシェイクでは、断られたときの本文は
/// 届いた分しか読めないので、これで見分ける
pub const REFUSAL_HEADER: &str = "x-gift-refusal";

/// 失敗した応答の本文から、Worker が断った理由を読む。OpenAI の `error` はオブジェクトなので読まない
pub fn refusal_reason(body: &str) -> Option<String> {
    serde_json::from_str::<CodeAnswer>(body).ok()?.error
}

/// 送り直しても断られるだけの応答なら、字幕を止める理由を返す（DESIGN.md 17.2）。
/// コードの 401 は理由を読まない（ハンドシェイクでは本文を読みきれず、ヘッダーもない）
pub fn refusal(
    access: &Access,
    status: StatusCode,
    reason: Option<&str>,
) -> Option<TranscriptionError> {
    match (status, reason) {
        (StatusCode::UNAUTHORIZED, _) if access.is_code() => Some(TranscriptionError::BetaRefused),
        (StatusCode::UNAUTHORIZED, _) => Some(TranscriptionError::InvalidKey {
            provider: access.provider(),
        }),
        // 403 の理由は Worker のもの。xAI の `error` も文字列なので、自分のキーでは読まない
        (StatusCode::FORBIDDEN, _) if !access.is_code() => None,
        (StatusCode::FORBIDDEN, Some(LIMIT)) => Some(TranscriptionError::BetaLimit),
        (StatusCode::FORBIDDEN, Some("revoked" | UNSUPPORTED)) => {
            Some(TranscriptionError::BetaRefused)
        }
        _ => None,
    }
}

pub async fn code_status(api: &str, code: &str) -> CodeStatus {
    let res = crate::http()
        .get(format!("{api}/gift"))
        .bearer_auth(code)
        .timeout(TIMEOUT)
        .send()
        .await;
    let res = match res {
        Ok(res) => res,
        Err(e) => {
            return CodeStatus::Unreachable {
                detail: format!("{e:#}"),
            };
        }
    };
    let status = res.status();
    let answer: CodeAnswer = match res.json().await {
        Ok(answer) => answer,
        Err(e) => {
            return CodeStatus::Unreachable {
                detail: format!("{status}: {e:#}"),
            };
        }
    };
    match (status, answer.error.as_deref()) {
        (StatusCode::OK, _) => CodeStatus::Active {
            providers: answer.providers,
        },
        (_, Some("revoked")) => CodeStatus::Revoked,
        (_, Some("limit")) => CodeStatus::Limit,
        (StatusCode::UNAUTHORIZED, _) => CodeStatus::Unknown,
        _ => CodeStatus::Unreachable {
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

/// 動いていない字幕を始めるときに、コードがまだ使えるか確かめる。使えれば中継する会社
pub async fn check_code(api: &str, code: &str) -> Result<Vec<Provider>, TranscriptionError> {
    match code_status(api, code).await {
        CodeStatus::Active { providers, .. } => Ok(providers),
        CodeStatus::Limit => Err(TranscriptionError::BetaLimit),
        CodeStatus::Revoked | CodeStatus::Unknown | CodeStatus::None => {
            Err(TranscriptionError::BetaRefused)
        }
        CodeStatus::Unreachable { detail } => Err(TranscriptionError::Failed {
            detail: format!("code: {detail}"),
        }),
    }
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
        let providers = check_code(&api, &code).await.unwrap();
        assert_eq!(providers, [Provider::Qwen, Provider::OpenRouter]);
        let access = |provider| Access::code(provider, code.clone(), &api);
        let meter = Arc::new(Meter::new(
            std::env::temp_dir().join("overlays-beta-test.json"),
        ));

        let wav = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../experiments/data/sample01.wav"
        );
        let request = BatchRequest {
            transcriber: &Transcriber::Api(access(Provider::Qwen)),
            model: "qwen-audio-3.1-asr-flash",
            hints: &Hints {
                language: "ja".into(),
                prompt: String::new(),
                keywords: vec!["ヘイズ".into()],
                heroes: Vec::new(),
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
            access(mt.provider),
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
        let beta = Access::code(Provider::Qwen, "code".into(), "http://worker");
        let refused = |reason| refusal(&beta, StatusCode::FORBIDDEN, Some(reason));
        assert_eq!(refused("limit"), Some(E::BetaLimit));
        assert_eq!(refused("revoked"), Some(E::BetaRefused));
        assert_eq!(
            refusal(&beta, StatusCode::UNAUTHORIZED, None),
            Some(E::BetaRefused)
        );
        // そのコードでは中継しない会社
        assert_eq!(refused(UNSUPPORTED), Some(E::BetaRefused));
    }

    #[test]
    fn a_code_at_its_daily_limit_stays_valid() {
        assert!(CodeStatus::Limit.is_valid());
        let active = CodeStatus::Active {
            providers: vec![Provider::Qwen],
        };
        assert!(active.is_valid());
        assert!(!CodeStatus::Revoked.is_valid());
        assert!(!CodeStatus::Unknown.is_valid());
    }

    #[test]
    fn only_answers_that_would_repeat_stop_the_subtitles() {
        let code = Access::code(Provider::Qwen, "code".into(), "http://worker");
        let key = Access::key(Provider::OpenAi, "sk".into());
        let other = Access::code(Provider::OpenRouter, "code".into(), "http://worker");
        let xai_key = Access::key(Provider::Xai, "xai".into());
        let refusal =
            |access, status, body| refusal(access, status, refusal_reason(body).as_deref());
        for (access, status, body, expected) in [
            (
                &code,
                StatusCode::UNAUTHORIZED,
                r#"{"error":"unknown"}"#,
                Some(TranscriptionError::BetaRefused),
            ),
            (
                &code,
                StatusCode::UNAUTHORIZED,
                "",
                Some(TranscriptionError::BetaRefused),
            ),
            (
                &code,
                StatusCode::FORBIDDEN,
                r#"{"error":"revoked"}"#,
                Some(TranscriptionError::BetaRefused),
            ),
            (
                &code,
                StatusCode::FORBIDDEN,
                r#"{"error":"limit"}"#,
                Some(TranscriptionError::BetaLimit),
            ),
            (
                &other,
                StatusCode::FORBIDDEN,
                r#"{"error":"unsupported"}"#,
                Some(TranscriptionError::BetaRefused),
            ),
            (
                &other,
                StatusCode::UNAUTHORIZED,
                "",
                Some(TranscriptionError::BetaRefused),
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
            (&code, StatusCode::BAD_REQUEST, r#"{"error":"limit"}"#, None),
            (
                &code,
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
            (&code, StatusCode::BAD_GATEWAY, "<html>", None),
        ] {
            assert_eq!(refusal(access, status, body), expected, "{status} {body}");
        }
    }

    #[tokio::test]
    async fn a_code_at_its_daily_limit_does_not_start() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let refuse = || async {
            (
                axum::http::StatusCode::FORBIDDEN,
                axum::Json(serde_json::json!({ "error": "limit" })),
            )
        };
        let router = axum::Router::new().route("/gift", axum::routing::get(refuse));
        tokio::spawn(async move { axum::serve(listener, router).await });
        assert_eq!(code_status(&api, "code").await, CodeStatus::Limit);
        assert_eq!(
            check_code(&api, "code").await,
            Err(TranscriptionError::BetaLimit)
        );
    }
}
