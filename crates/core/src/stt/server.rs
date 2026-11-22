//! 開発者のサーバーの仕事の口（`/v2`。DESIGN.md 17.4）に、文字起こしを頼む。どのモデルで作るかはサーバーが決める。
//! かかった額は数えない（払うのは開発者）。
//!
//! 確定は 1 回の HTTP（`/transcribe`）。途中の字幕は WebSocket（`/live`）に 16kHz の PCM を流し、時刻の付いた文を
//! 受け取って、アプリの発話に割り当てる（`timed.rs`）

use super::*;
use crate::access::Server;

/// 確定の答えを待つ時間。サーバーは 15 秒で諦める
const TIMEOUT: Duration = Duration::from_secs(20);

/// サーバーが受け取る言葉の数と、1 つの長さ（UTF-16 の単位。server/src/v2.ts の `MAX_HINTS`、`MAX_HINT_CHARS`）。
/// 越えると要求ごと断られるので、先のものから入るだけ送る（先に大事な言葉が並んでいる。`subtitles::keywords`）
const MAX_HINTS: usize = 200;
const MAX_HINT_CHARS: usize = 100;

/// サーバーが断った応答なら、その理由。どの工程も同じに断られるもの（コードが使えない、今日の上限、古い版）は、
/// 字幕を止める理由にする。その仕事だけを止めているときと、混んでいるときは、ふつうの失敗
fn refused(status: StatusCode, reason: Option<&str>) -> anyhow::Error {
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

/// 1 セッション分。入力が尽きたら Ok、切れたら Err で返る（`super::realtime` と同じ決まり）
pub(super) async fn live(
    server: &Server,
    hints: &Hints,
    speaking: &mut Speaking,
    input: &mut mpsc::UnboundedReceiver<Input>,
    deltas: &mpsc::Sender<Delta>,
) -> anyhow::Result<()> {
    let socket = tokio::time::timeout(HANDSHAKE_TIMEOUT, connect(server, hints))
        .await
        .context("realtime handshake timed out")??;
    let finish = json!({ "type": "finish" });
    let read = |event: &Value| match event["type"].as_str() {
        Some("sentence") => Ok(Some(serde_json::from_value(event.clone())?)),
        _ => Ok(None),
    };
    timed::stream(socket, finish.to_string(), read, speaking, input, deltas).await
}

/// つないで、サーバーが聞き始める（`ready`）のを待つ。音声は、そのあとに流す
async fn connect(server: &Server, hints: &Hints) -> anyhow::Result<(Sink, Stream)> {
    let url = server
        .url(&format!("/live?language={}", hints.language))
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", server.code()))?,
    );
    let (ws, _) = match tokio_tungstenite::connect_async(request).await {
        Ok(connected) => connected,
        // 断られたときの本文は読みきれないので、理由はヘッダーで読む
        Err(tungstenite::Error::Http(res)) => {
            let reason = res.headers().get(relay::REFUSAL_HEADER);
            let reason = reason.and_then(|reason| reason.to_str().ok());
            return Err(refused(res.status(), reason));
        }
        Err(e) => return Err(e).context("connect realtime"),
    };
    let (sink, mut stream) = ws.split();
    while let Some(msg) = stream.next().await {
        let Message::Text(text) = msg? else { continue };
        let event: Value = serde_json::from_str(&text)?;
        if event["type"] == "ready" {
            return Ok((sink, stream));
        }
    }
    bail!("realtime closed before ready")
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

    /// サーバーが聞き始めてから音声を流し、届いた文を、時刻で発話に割り当てる。入力が尽きたら、終わりを知らせる
    #[tokio::test]
    #[allow(clippy::result_large_err)]
    async fn the_server_writes_while_it_listens() {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let served = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let check = |request: &Request, response: Response| {
                assert_eq!(request.uri(), "/v2/live?language=ja");
                assert_eq!(request.headers()["authorization"], "Bearer code");
                Ok(response)
            };
            let mut ws = tokio_tungstenite::accept_hdr_async(socket, check)
                .await
                .unwrap();
            let send = |event: Value| Message::text(event.to_string());
            ws.send(send(json!({ "type": "ready" }))).await.unwrap();
            // 音声が届いたら、文を返す
            let audio = ws.next().await.unwrap().unwrap();
            assert!(audio.is_binary());
            let words =
                json!([{ "at": 100, "text": "さあ" }, { "at": 400, "text": "始まりました。" }]);
            let sentence =
                json!({ "type": "sentence", "id": 1, "begin": 100, "text": "", "words": words });
            ws.send(send(sentence)).await.unwrap();
            loop {
                match ws.next().await.unwrap().unwrap() {
                    Message::Text(text) => return text.to_string(),
                    _ => continue,
                }
            }
        });
        let (input_tx, mut input) = mpsc::unbounded_channel();
        let (deltas_tx, mut deltas) = mpsc::channel(8);
        let server = Server::new(&api, "code".into());
        let hints = hints();
        let session = tokio::spawn(async move {
            let mut speaking = Speaking::default();
            live(&server, &hints, &mut speaking, &mut input, &deltas_tx).await
        });
        input_tx.send(Input::Open("a".into())).unwrap();
        input_tx.send(Input::Audio(vec![0.1; 1600])).unwrap();
        let delta = deltas.recv().await.unwrap();
        assert_eq!(
            (delta.segment.as_str(), delta.text.as_str(), delta.whole),
            ("a", "さあ始まりました。", true)
        );
        drop(input_tx);
        session.await.unwrap().unwrap();
        assert_eq!(
            served.await.unwrap(),
            json!({ "type": "finish" }).to_string()
        );
    }
}
