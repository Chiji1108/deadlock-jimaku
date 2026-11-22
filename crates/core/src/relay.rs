//! 開発者のサーバー（Worker）に仕事を頼むコード（DESIGN.md 17.2、17.4）。ベータのコードをもらう、まだ使えるか確かめる、
//! 断られた理由を読む。0.10.0 までは、コードを会社のキーの代わりにして、サーバーが会社の形のまま中継していた

use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::access::{Access, Server};
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
    Active,
    /// 止められた
    Revoked,
    /// 今日（UTC）の上限に達した
    Limit,
    /// ないコード
    Unknown,
    /// この版のアプリでは、もう中継しない（更新が要る）
    Outdated,
    /// サーバーにつながらない
    Unreachable {
        detail: String,
    },
}

impl CodeStatus {
    /// 持っていてよいコードか。今日の上限は日が変われば戻り、古い版はアプリを更新すれば使えるので、今は使えなくても有効
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Active { .. } | Self::Limit | Self::Outdated)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodeAnswer {
    #[serde(default)]
    error: Option<String>,
}

/// サーバーが今日の上限で断るときの理由（JSON の `error`）
pub const LIMIT: &str = "limit";

/// サーバーが、その仕事だけを止めているとき（仕事の口。DESIGN.md 17.4）の理由。ほかの工程は続ける
pub const STOPPED: &str = "stopped";

/// この版のアプリでは、もう中継しないとき（古い口をやめたあと）の理由。0.9.7 から読む
pub const OUTDATED: &str = "outdated";

/// サーバーに伝える、アプリの版と OS（`deadlock-jimaku/0.9.7 (windows)`）。コードを確かめるときと、もらうときに
/// 付ける。サーバーは、どの版がまだ使われているかをコードごとに持ち、古い口をいつやめられるかを見る（DESIGN.md 17.2）。
/// 中継のほかの要求と、会社へ直接送る要求には付けない
fn app() -> String {
    format!(
        "deadlock-jimaku/{} ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    )
}

/// 失敗した応答の本文から、Worker が断った理由を読む。OpenAI の `error` はオブジェクトなので読まない
pub fn refusal_reason(body: &str) -> Option<String> {
    serde_json::from_str::<CodeAnswer>(body).ok()?.error
}

/// 会社が鍵を断った応答なら、字幕を止める理由を返す（送り直しても断られるだけ）
pub fn refusal(access: &Access, status: StatusCode) -> Option<TranscriptionError> {
    (status == StatusCode::UNAUTHORIZED).then_some(TranscriptionError::InvalidKey {
        provider: access.provider(),
    })
}

/// Worker が、コードを見て断った理由（どの仕事の口でも同じ）。どの工程も同じに断られるので、字幕を止める
/// （止めずに、サーバーを使わない作り方で続けるかは `Store::fall_back`）。その仕事だけを止めているとき
/// （[`STOPPED`]）と、混んでいるときは、ここでは止めない
pub fn code_refusal(status: StatusCode, reason: Option<&str>) -> Option<TranscriptionError> {
    match (status, reason) {
        (StatusCode::UNAUTHORIZED, _) => Some(TranscriptionError::BetaRefused),
        (StatusCode::FORBIDDEN, Some(LIMIT)) => Some(TranscriptionError::BetaLimit),
        (StatusCode::FORBIDDEN, Some(OUTDATED)) => Some(TranscriptionError::BetaOutdated),
        (StatusCode::FORBIDDEN, Some("revoked")) => Some(TranscriptionError::BetaRefused),
        _ => None,
    }
}

pub async fn code_status(api: &str, code: &str) -> CodeStatus {
    let res = crate::http()
        .get(Server::new(api, code.to_owned()).url("/status"))
        .bearer_auth(code)
        .header(reqwest::header::USER_AGENT, app())
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
        (StatusCode::OK, _) => CodeStatus::Active,
        (_, Some("revoked")) => CodeStatus::Revoked,
        (_, Some("limit")) => CodeStatus::Limit,
        (_, Some(OUTDATED)) => CodeStatus::Outdated,
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
        .header(reqwest::header::USER_AGENT, app())
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

/// 動いていない字幕を始めるときに、コードがまだ使えるか確かめる
pub async fn check_code(api: &str, code: &str) -> Result<(), TranscriptionError> {
    match code_status(api, code).await {
        CodeStatus::Active => Ok(()),
        CodeStatus::Limit => Err(TranscriptionError::BetaLimit),
        CodeStatus::Outdated => Err(TranscriptionError::BetaOutdated),
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
    use crate::secrets::Provider;

    /// Worker に、ベータのコードで、アプリの部品が実際に送る頼みごとを通す。手元の Worker（`wrangler dev`。偽の開発者のキー）なら、
    /// 形が Worker の検査を通れば外に送り、偽のキーなので「外の不調」（502）が返る（400 なら形が合っていない）。
    /// 本番（`https://api.jimaku.deadlock.jp/v1`）なら、文字起こしと、選んだ用語、訳が返る:
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
        // コードは 1 日に同じ所から 3 つまでなので、持っていれば `BETA_TEST_CODE` で渡す
        let code = match std::env::var("BETA_TEST_CODE") {
            Ok(code) => code,
            Err(_) => join_beta(&api).await.unwrap(),
        };
        check_code(&api, &code).await.unwrap();
        let server = Server::new(&api, code);
        let meter = Arc::new(Meter::new(
            std::env::temp_dir().join("overlays-beta-test.json"),
        ));
        // 外が断ったら、Worker は「外の不調」（502）として返す
        let upstream = |e: &anyhow::Error| assert!(format!("{e:#}").contains("upstream"), "{e:#}");

        let wav = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../experiments/data/sample01.wav"
        );
        let request = BatchRequest {
            transcriber: &Transcriber::Server(server.clone()),
            model: "",
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
            Err(e) => upstream(&e),
        }

        let state = serde_json::json!("Transcript: ヘイズ強い");
        let questions = serde_json::json!([{
            "name": "c0",
            "instructions": "Which one is meant by ヘイズ?",
            "choices": [
                { "value": "hero:13", "description": "Haze: Deadlock hero" },
                { "value": "none", "description": "not a game term" },
            ],
        }]);
        match crate::decide::ask_server(crate::http(), &server, &state, &questions).await {
            Ok(chosen) => println!("decision: {chosen:?}"),
            Err(e) => upstream(&e),
        }

        assert!(Engines::BETA.server());
        let translator =
            crate::mt::Translator::server(server, "ja".into(), vec!["en".into()], meter.clone());
        let spans = [Span::Text("強い".into())];
        // 翻訳の失敗は、その文の訳を諦めるだけ（エラーにならず、訳が空になる）。外が断ったことは、Worker の記録で見る
        let translated = translator.translate(crate::http(), &spans, None).await;
        println!("translation: {:?}", translated.unwrap());
    }

    #[test]
    fn a_refused_beta_code_gets_its_own_reason() {
        use TranscriptionError as E;
        let refused = |reason| code_refusal(StatusCode::FORBIDDEN, Some(reason));
        assert_eq!(refused("limit"), Some(E::BetaLimit));
        assert_eq!(refused("revoked"), Some(E::BetaRefused));
        assert_eq!(
            code_refusal(StatusCode::UNAUTHORIZED, None),
            Some(E::BetaRefused)
        );
        // 古い口をやめたあとの、この版への答え
        assert_eq!(refused(OUTDATED), Some(E::BetaOutdated));
        // その仕事だけを止めているときと、混んでいるときは、字幕を止めない
        assert_eq!(refused(STOPPED), None);
        assert_eq!(
            code_refusal(StatusCode::TOO_MANY_REQUESTS, Some("busy")),
            None
        );
    }

    #[test]
    fn a_code_at_its_daily_limit_stays_valid() {
        assert!(CodeStatus::Limit.is_valid());
        assert!(CodeStatus::Active.is_valid());
        assert!(!CodeStatus::Revoked.is_valid());
        assert!(!CodeStatus::Unknown.is_valid());
    }

    #[test]
    fn only_a_refused_key_stops_the_subtitles() {
        let key = Access::key(Provider::OpenAi, "sk".into());
        assert_eq!(
            refusal(&key, StatusCode::UNAUTHORIZED),
            Some(TranscriptionError::InvalidKey {
                provider: Provider::OpenAi,
            })
        );
        for status in [StatusCode::FORBIDDEN, StatusCode::TOO_MANY_REQUESTS] {
            assert_eq!(refusal(&key, status), None);
        }
        // 会社の `error` はオブジェクトのことがある（OpenAI）。Worker の理由としては読まない
        assert_eq!(refusal_reason(r#"{"error":{"message":"no"}}"#), None);
        assert_eq!(
            refusal_reason(r#"{"error":"limit"}"#).as_deref(),
            Some(LIMIT)
        );
    }
}
