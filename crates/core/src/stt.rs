//! OpenAI の音声認識（DESIGN.md 3 章）。リアルタイム（GPT-Live-Transcribe）とバッチ（GPT-Transcribe）。
//!
//! リアルタイムはサーバー側で発話を区切らない。こちらの VAD で発話を閉じたら commit し、
//! そこまでの音声が 1 つの item になる。delta には最初から item の ID が付いて届く。

use std::collections::VecDeque;

use anyhow::{Context, bail};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

use crate::audio::{Resampler, SAMPLE_RATE, to_i16};

const REALTIME_URL: &str = "wss://api.openai.com/v1/realtime?intent=transcription";
const BATCH_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
/// リアルタイムの `audio/pcm` は 24kHz
const REALTIME_RATE: u32 = 24_000;
/// input_audio_buffer.append 1 回で送る長さ（100ms）
const APPEND_SAMPLES: usize = REALTIME_RATE as usize / 10;
/// これより短い音声は commit できない（空の発話になる）
const MIN_COMMIT_SAMPLES: usize = REALTIME_RATE as usize / 10;

/// リアルタイムとバッチで共通の手がかり
pub struct Hints {
    /// 話す言語（ISO 639-1）
    pub language: String,
    /// 録音の説明
    pub prompt: String,
    /// 出てきそうな用語（DESIGN.md 8.4）
    pub keywords: Vec<String>,
}

pub struct RealtimeConfig {
    pub api_key: String,
    pub model: String,
    /// `minimal` / `low` / `medium` / `high` / `xhigh`（低いほど早く、高いほど正確）
    pub delay: String,
}

/// リアルタイムに送るもの。同じチャンネルで送り、音声と commit の順番を保つ
pub enum Input {
    Audio(Vec<f32>),
    /// ここまでの音声を 1 つの発話にする
    Commit,
}

pub enum Event {
    Delta {
        item: String,
        text: String,
    },
    /// commit した順に 1 つずつ届く。音声が短すぎて commit しなかったものは None
    Committed(Option<String>),
    /// 接続が切れて張り直した。前のセッションの item や commit の確認はもう届かない
    Reconnected,
}

/// 1 セッション分。入力が尽きたら、commit の確認を待って Ok、切れたら Err で返る
pub async fn realtime(
    cfg: &RealtimeConfig,
    hints: &Hints,
    input: &mut mpsc::UnboundedReceiver<Input>,
    events: &mpsc::Sender<Event>,
) -> anyhow::Result<()> {
    let mut request = REALTIME_URL.into_client_request()?;
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
            "type": "transcription",
            "audio": { "input": {
                "format": { "type": "audio/pcm", "rate": REALTIME_RATE },
                "transcription": {
                    "model": cfg.model,
                    "languages": [hints.language],
                    "prompt": hints.prompt,
                    "keywords": hints.keywords,
                    "delay": cfg.delay,
                },
                "turn_detection": null,
            }},
        }
    });
    sink.send(Message::text(update.to_string())).await?;
    expect_event(&mut stream, "session.updated").await?;
    tracing::info!("realtime session started");

    let mut resampler = Resampler::new(SAMPLE_RATE, REALTIME_RATE)?;
    let mut pcm: Vec<u8> = Vec::with_capacity(APPEND_SAMPLES * 2);
    // 前の commit から送った音声の長さ
    let mut buffered = 0;
    // commit したもの（true）と、短すぎて commit しなかったもの（false）を順に並べる。
    // 確認は commit した順に届くので、commit しなかったものはその順番で None にする
    let mut commits: VecDeque<bool> = VecDeque::new();
    let mut input_done = false;
    loop {
        if input_done && commits.is_empty() {
            return Ok(());
        }
        tokio::select! {
            next = input.recv(), if !input_done => {
                let commit = match next {
                    Some(Input::Audio(chunk)) => {
                        let samples = resampler.process(&chunk);
                        buffered += samples.len();
                        pcm.extend(samples.iter().flat_map(|s| to_i16(*s).to_le_bytes()));
                        if pcm.len() >= APPEND_SAMPLES * 2 {
                            sink.send(append(&mut pcm)).await?;
                        }
                        false
                    }
                    Some(Input::Commit) => true,
                    None => {
                        input_done = true;
                        buffered >= MIN_COMMIT_SAMPLES
                    }
                };
                if commit {
                    if !pcm.is_empty() {
                        sink.send(append(&mut pcm)).await?;
                    }
                    let sent = buffered >= MIN_COMMIT_SAMPLES;
                    if sent {
                        sink.send(Message::text(json!({ "type": "input_audio_buffer.commit" }).to_string())).await?;
                    }
                    buffered = 0;
                    commits.push_back(sent);
                    flush_skipped(&mut commits, events).await;
                }
            }
            msg = stream.next() => {
                let event = match msg {
                    Some(Ok(Message::Text(text))) => serde_json::from_str::<Value>(&text)?,
                    Some(Ok(Message::Close(frame))) => bail!("realtime closed: {frame:?}"),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(e.into()),
                    None => bail!("realtime stream ended"),
                };
                let item = event["item_id"].as_str().map(str::to_owned);
                match event["type"].as_str() {
                    Some("conversation.item.input_audio_transcription.delta") => {
                        if let (Some(item), Some(text)) = (item, event["delta"].as_str()) {
                            let _ = events.send(Event::Delta { item, text: text.to_owned() }).await;
                        }
                    }
                    Some("input_audio_buffer.committed") => {
                        commits.pop_front();
                        let _ = events.send(Event::Committed(item)).await;
                        flush_skipped(&mut commits, events).await;
                    }
                    // 1 つの要求の失敗（短すぎる commit など）ではセッションは続く
                    Some("error") => tracing::warn!("realtime error: {}", event["error"]),
                    _ => {}
                }
            }
        }
    }
}

/// 先頭に並んだ「commit しなかったもの」を None として知らせる
async fn flush_skipped(commits: &mut VecDeque<bool>, events: &mpsc::Sender<Event>) {
    while commits.front() == Some(&false) {
        commits.pop_front();
        let _ = events.send(Event::Committed(None)).await;
    }
}

/// 溜めた PCM を input_audio_buffer.append にして空にする
fn append(pcm: &mut Vec<u8>) -> Message {
    let audio = base64::engine::general_purpose::STANDARD.encode(&*pcm);
    pcm.clear();
    Message::text(json!({ "type": "input_audio_buffer.append", "audio": audio }).to_string())
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
    pub hints: &'a Hints,
    pub wav: Vec<u8>,
}

/// 発話区間の WAV を文字起こしする
pub async fn transcribe(http: &reqwest::Client, req: BatchRequest<'_>) -> anyhow::Result<String> {
    let file = reqwest::multipart::Part::bytes(req.wav)
        .file_name("segment.wav")
        .mime_str("audio/wav")?;
    let mut form = reqwest::multipart::Form::new()
        .text("model", req.model.to_owned())
        .text("prompt", req.hints.prompt.clone())
        .text("languages[]", req.hints.language.clone())
        .part("file", file);
    for keyword in &req.hints.keywords {
        form = form.text("keywords[]", keyword.clone());
    }
    let res = http
        .post(BATCH_URL)
        .bearer_auth(req.api_key)
        .multipart(form)
        .send()
        .await?;
    let status = res.status();
    if !status.is_success() {
        // 400 の理由は本文にしか書かれていない
        bail!("{status}: {}", res.text().await.unwrap_or_default());
    }
    let res: Value = res.json().await?;
    res["text"]
        .as_str()
        .map(|t| t.trim().to_owned())
        .context("transcription response has no text")
}
