//! 開発者のサーバーの仕事の口（`/v2`。DESIGN.md 17.4）に、文字起こしを頼む。どのモデルで作るかはサーバーが決める。
//! かかった額は数えない（払うのは開発者）。
//!
//! 確定は 1 回の HTTP（`/transcribe`）。途中の字幕は頼まない（サーバーには `/live` の口があるが、無料で頼めるのは
//! 「おすすめ」の中身だけにしている。0.10.0 の rc.3 だけ、アプリから頼んでいた）

use super::*;
use crate::access::Server;

/// 確定の答えを待つ時間。サーバーは 15 秒で諦める
const TIMEOUT: Duration = Duration::from_secs(20);

/// サーバーが受け取る言葉の数と、1 つの長さ（UTF-16 の単位。server/src/v2.ts の `MAX_HINTS`、`MAX_HINT_CHARS`）。
/// 越えると要求ごと断られるので、先のものから入るだけ送る（先に大事な言葉が並んでいる。`subtitles::keywords`）
const MAX_HINTS: usize = 200;
const MAX_HINT_CHARS: usize = 100;

/// サーバーが断った応答なら、その理由。どの工程も同じに断られるもの（コードが使えない、今日の上限、古い版）は、
/// 字幕を止める理由にする。サーバーが文字起こしを止めているとき（その仕事だけを止めた、ベータを閉じた）も同じ:
/// 翻訳や用語と違って、文字起こしがないと字幕そのものが出ないので、黙って続けず、サーバーを使わない作り方に
/// 替える（`Store::fall_back`）。混んでいるときと外の不調は、その 1 回の失敗
fn refused(status: StatusCode, reason: Option<&str>) -> anyhow::Error {
    if reason == Some(relay::STOPPED) {
        return TranscriptionError::BetaRefused.into();
    }
    match relay::code_refusal(status, reason) {
        Some(refused) => refused.into(),
        None => anyhow::anyhow!("server: {status}: {}", reason.unwrap_or_default()),
    }
}

/// 発話区間の WAV を文字起こしする
pub(super) async fn transcribe(
    http: &reqwest::Client,
    server: &Server,
    req: BatchRequest<'_>,
) -> anyhow::Result<String> {
    let fits = |hint: &&String| hint.encode_utf16().count() <= MAX_HINT_CHARS;
    let hints: Vec<&String> = (req.hints.keywords.iter().filter(fits))
        .take(MAX_HINTS)
        .collect();
    let body = json!({
        "audio": base64::engine::general_purpose::STANDARD.encode(&req.wav),
        "language": req.hints.language,
        "hints": hints,
    });
    let res = http
        .post(server.url("/transcribe"))
        .bearer_auth(server.code())
        .timeout(TIMEOUT)
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(refused(status, relay::refusal_reason(&body).as_deref()));
    }
    text(&res.json().await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    fn hints() -> Hints {
        // サーバーが受け取らない長さの言葉と、数を越えた分は送らない
        let mut keywords = vec!["x".repeat(101), "ヘイズ".into()];
        keywords.extend((0..300).map(|i| format!("用語{i}")));
        Hints {
            language: "ja".into(),
            prompt: "x".into(),
            keywords,
            heroes: Vec::new(),
        }
    }

    /// 音声と、話す言語、出てきそうな言葉だけを送る（モデルは送らない）。料金は数えない
    #[tokio::test]
    async fn the_server_writes_what_was_said() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let answer = |headers: axum::http::HeaderMap, axum::Json(asked): axum::Json<Value>| async move {
            assert_eq!(headers["authorization"], "Bearer code");
            let wav = base64::engine::general_purpose::STANDARD.encode([1, 2, 3]);
            let hints = asked["hints"].as_array().unwrap();
            assert_eq!(
                (hints.len(), &hints[0], &hints[199]),
                (200, &json!("ヘイズ"), &json!("用語198"))
            );
            assert_eq!(
                (&asked["audio"], &asked["language"]),
                (&json!(wav), &json!("ja"))
            );
            assert_eq!(asked.as_object().unwrap().len(), 3);
            axum::Json(json!({ "text": " ヘイズ強い " }))
        };
        let router = axum::Router::new().route("/v2/transcribe", axum::routing::post(answer));
        tokio::spawn(async move { axum::serve(listener, router).await });
        let dir = tempfile::tempdir().unwrap();
        let meter = Meter::new(dir.path().join("usage.json"));
        let request = BatchRequest {
            transcriber: &Transcriber::Server(Server::new(&api, "code".into())),
            model: "",
            hints: &hints(),
            wav: vec![1, 2, 3],
            seconds: 1.0,
            meter: &meter,
        };
        let text = super::super::transcribe(crate::http(), request).await;
        assert_eq!(text.unwrap(), "ヘイズ強い");
        assert!(meter.watch().borrow().session.is_empty());
    }
}
