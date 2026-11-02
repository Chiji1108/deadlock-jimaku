//! Mistral Voxtral の Realtime（WebSocket）とバッチ文字起こし（DESIGN.md 3 章）。

use anyhow::{Context, bail};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

use crate::audio::{SAMPLE_RATE, to_i16};

const REALTIME_URL: &str = "wss://api.mistral.ai/v1/audio/transcriptions/realtime";
const BATCH_URL: &str = "https://api.mistral.ai/v1/audio/transcriptions";
/// input_audio.append 1 回で送る長さ（100ms）
const APPEND_SAMPLES: usize = SAMPLE_RATE as usize / 10;

pub struct RealtimeConfig {
    pub api_key: String,
    pub model: String,
    pub language: String,
    pub context_bias: Vec<String>,
    pub target_streaming_delay_ms: u32,
}

/// 1 セッション分。音声を送り、届いた delta を流す。音声が尽きたら Ok、切れたら Err で返る。
/// 設定は最初の音声より前に 1 回しか送れないので、張り直すときはこの関数を呼び直す。
pub async fn realtime(
    cfg: &RealtimeConfig,
    audio: &mut mpsc::Receiver<Vec<f32>>,
    deltas: &mpsc::Sender<String>,
) -> anyhow::Result<()> {
    let mut request = format!("{REALTIME_URL}?model={}", cfg.model).into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", cfg.api_key))?,
    );
    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .context("connect realtime")?;
    let (mut sink, mut stream) = ws.split();

    expect_event(&mut stream, "session.created").await?;
    let update = json!({
        "type": "session.update",
        "session": {
            "audio_format": { "encoding": "pcm_s16le", "sample_rate": SAMPLE_RATE },
            "target_streaming_delay_ms": cfg.target_streaming_delay_ms,
            "language": cfg.language,
            "context_bias": cfg.context_bias,
        }
    });
    sink.send(Message::text(update.to_string())).await?;
    expect_event(&mut stream, "session.updated").await?;
    tracing::info!("realtime session started");

    let mut pcm: Vec<u8> = Vec::with_capacity(APPEND_SAMPLES * 2);
    let mut audio_done = false;
    loop {
        tokio::select! {
            chunk = audio.recv(), if !audio_done => match chunk {
                Some(chunk) => {
                    pcm.extend(chunk.iter().flat_map(|s| to_i16(*s).to_le_bytes()));
                    if pcm.len() >= APPEND_SAMPLES * 2 {
                        sink.send(append(&mut pcm)).await?;
                    }
                }
                None => {
                    audio_done = true;
                    // 100ms に満たない残りも送ってから終える
                    if !pcm.is_empty() {
                        sink.send(append(&mut pcm)).await?;
                    }
                    sink.send(Message::text(json!({ "type": "input_audio.end" }).to_string())).await?;
                }
            },
            msg = stream.next() => {
                let event = match msg {
                    Some(Ok(Message::Text(text))) => serde_json::from_str::<Value>(&text)?,
                    Some(Ok(Message::Close(frame))) => bail!("realtime closed: {frame:?}"),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(e.into()),
                    None => bail!("realtime stream ended"),
                };
                match event["type"].as_str() {
                    Some("transcription.text.delta") => {
                        if let Some(text) = event["text"].as_str() {
                            let _ = deltas.send(text.to_owned()).await;
                        }
                    }
                    Some("transcription.done") => return Ok(()),
                    Some("error") => bail!("realtime error: {}", event["error"]),
                    _ => {}
                }
            }
        }
    }
}

/// 溜めた PCM を input_audio.append にして空にする
fn append(pcm: &mut Vec<u8>) -> Message {
    let audio = base64::engine::general_purpose::STANDARD.encode(&*pcm);
    pcm.clear();
    Message::text(json!({ "type": "input_audio.append", "audio": audio }).to_string())
}

async fn expect_event<S>(stream: &mut S, expected: &str) -> anyhow::Result<()>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while let Some(msg) = stream.next().await {
        let Message::Text(text) = msg? else { continue };
        let event: Value = serde_json::from_str(&text)?;
        match event["type"].as_str() {
            Some(t) if t == expected => return Ok(()),
            Some("error") => bail!("realtime error: {}", event["error"]),
            _ => {}
        }
    }
    bail!("realtime closed before {expected}")
}

pub struct BatchRequest<'a> {
    pub api_key: &'a str,
    pub model: &'a str,
    pub language: &'a str,
    pub context_bias: &'a [String],
    pub wav: Vec<u8>,
}

/// 発話区間の WAV を文字起こしする
pub async fn transcribe(http: &reqwest::Client, req: BatchRequest<'_>) -> anyhow::Result<String> {
    let file = reqwest::multipart::Part::bytes(req.wav)
        .file_name("segment.wav")
        .mime_str("audio/wav")?;
    let mut form = reqwest::multipart::Form::new()
        .text("model", req.model.to_owned())
        .text("language", req.language.to_owned())
        .part("file", file);
    for term in req.context_bias {
        form = form.text("context_bias", term.clone());
    }
    let res = http
        .post(BATCH_URL)
        .bearer_auth(req.api_key)
        .multipart(form)
        .send()
        .await?;
    let status = res.status();
    if !status.is_success() {
        // 400 の理由（context_bias が長すぎる、など）は本文にしか書かれていない
        bail!("{status}: {}", res.text().await.unwrap_or_default());
    }
    let res: Value = res.json().await?;
    res["text"]
        .as_str()
        .map(|t| t.trim().to_owned())
        .context("transcription response has no text")
}
