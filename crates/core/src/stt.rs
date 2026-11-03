//! OpenAI の音声認識（DESIGN.md 3 章）。リアルタイム（GPT-Live-Transcribe）とバッチ（GPT-Transcribe）。
//!
//! リアルタイムはサーバー側で発話を区切らない。こちらの VAD で発話を開いたら音声を送り始め、
//! 閉じたら commit する。commit までの音声が 1 つの item になり、delta には item の ID が付いて届く。
//! item と発話の対応は 1 つの接続の中だけで持つ（張り直したら作り直す）。

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

use crate::audio::{SAMPLE_RATE, to_i16};

const REALTIME_URL: &str = "wss://api.openai.com/v1/realtime?intent=transcription";
const BATCH_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
/// リアルタイムの `audio/pcm` は 24kHz（入力の 16kHz からちょうど 3/2 倍）
const REALTIME_RATE: u32 = 24_000;
/// input_audio_buffer.append 1 回で送る長さ（100ms）
const APPEND_SAMPLES: usize = REALTIME_RATE as usize / 10;
/// これより短い音声は commit できない（空の発話になる）
const MIN_COMMIT_SAMPLES: usize = REALTIME_RATE as usize / 10;
/// 発話を開く前の音声も少し送る（VAD が開くのは話し始めより少し遅い）
const PREROLL_SAMPLES: usize = SAMPLE_RATE as usize * 3 / 10;
/// 接続と設定にかける時間の上限（止まったまま音声を溜めない）
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

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

/// リアルタイムに送るもの。同じチャンネルで送り、順番を保つ
pub enum Input {
    /// 発話を開いた（Segment の ID）。ここから閉じるまでの音声がこの発話になる
    Open(String),
    Audio(Vec<f32>),
    /// 今の発話を閉じた
    Close,
}

/// 発話（Segment の ID）に追記する文字
pub struct Delta {
    pub segment: String,
    pub text: String,
}

/// 接続をまたいで持つもの（今の発話と、開く前の音声）
#[derive(Default)]
pub struct Speaking {
    current: Option<String>,
    preroll: VecDeque<f32>,
}

impl Speaking {
    /// 送らずに読み捨てる入力（切れている間・接続している間）で、状態だけ進める
    pub fn skip(&mut self, input: Input) {
        match input {
            Input::Open(id) => self.current = Some(id),
            Input::Audio(chunk) => {
                if self.current.is_none() {
                    self.remember(&chunk);
                }
            }
            Input::Close => {
                self.current = None;
                // 閉じた発話の終わりを、次の発話の頭として送らない
                self.preroll.clear();
            }
        }
    }

    /// 話していない間の音声だけを、発話を開く前の分として持つ
    fn remember(&mut self, chunk: &[f32]) {
        self.preroll.extend(chunk);
        let extra = self.preroll.len().saturating_sub(PREROLL_SAMPLES);
        self.preroll.drain(..extra);
    }
}

/// 1 つの接続の中の、item と発話の対応
#[derive(Default)]
struct Items {
    /// item の ID → 発話
    by_item: HashMap<String, String>,
    /// commit したが、まだ item の ID の確認が届いていない発話（commit した順）
    committing: VecDeque<String>,
}

impl Items {
    /// delta の届け先。初めて見る item は、確認待ちの commit 済みの発話（先に commit したもの）、
    /// なければ今話している発話のものと見なす（item は commit した順に並ぶ）
    fn segment(&mut self, item: &str, current: Option<&String>) -> Option<String> {
        if let Some(segment) = self.by_item.get(item) {
            return Some(segment.clone());
        }
        let segment = self.committing.front().or(current)?.clone();
        self.by_item.insert(item.to_owned(), segment.clone());
        Some(segment)
    }

    /// commit の確認で、item を commit した順の発話に付け直す（見なしが違っていても、ここで正しくなる）
    fn committed(&mut self, item: String) {
        if let Some(segment) = self.committing.pop_front() {
            self.by_item.insert(item, segment);
        }
    }

    /// 文字が届き終わった item は、もう delta が来ないので対応を消す
    fn completed(&mut self, item: &str) {
        self.by_item.remove(item);
    }

    /// commit しなかった・断られた発話の対応を消す（サーバー側でもその音声を消す）
    fn drop_segment(&mut self, segment: &str) {
        self.committing.retain(|s| s != segment);
        self.by_item.retain(|_, s| s != segment);
    }
}

/// 1 セッション分。入力が尽きたら Ok（最後の発話の確定はバッチで取る）、切れたら Err で返る
pub async fn realtime(
    cfg: &RealtimeConfig,
    hints: &Hints,
    speaking: &mut Speaking,
    input: &mut mpsc::UnboundedReceiver<Input>,
    deltas: &mpsc::Sender<Delta>,
) -> anyhow::Result<()> {
    let (mut sink, mut stream) = tokio::time::timeout(HANDSHAKE_TIMEOUT, connect(cfg, hints))
        .await
        .context("realtime handshake timed out")??;
    tracing::info!("realtime session started");

    // 接続している間に溜まった入力は送らない（古い言葉が今の発話に混ざる）。今の発話の続きから送る
    while let Ok(next) = input.try_recv() {
        speaking.skip(next);
    }
    let mut upsampler = Upsampler::default();
    let mut pcm: Vec<u8> = Vec::with_capacity(APPEND_SAMPLES * 2);
    // 今の発話で送った音声の長さ
    let mut buffered = 0;
    if speaking.current.is_some() {
        let preroll: Vec<f32> = speaking.preroll.drain(..).collect();
        buffered += push(&mut upsampler, &mut pcm, &preroll);
    }
    let mut items = Items::default();
    loop {
        tokio::select! {
            next = input.recv() => match next {
                Some(Input::Open(id)) => {
                    speaking.current = Some(id);
                    let preroll: Vec<f32> = speaking.preroll.drain(..).collect();
                    buffered += push(&mut upsampler, &mut pcm, &preroll);
                }
                Some(Input::Audio(chunk)) => {
                    if speaking.current.is_some() {
                        buffered += push(&mut upsampler, &mut pcm, &chunk);
                        if pcm.len() >= APPEND_SAMPLES * 2 {
                            sink.send(append(&mut pcm)).await?;
                        }
                    } else {
                        speaking.remember(&chunk);
                    }
                }
                Some(Input::Close) => {
                    close(&mut sink, speaking, &mut items, &mut pcm, buffered).await?;
                    buffered = 0;
                }
                None => {
                    close(&mut sink, speaking, &mut items, &mut pcm, buffered).await?;
                    return Ok(());
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
                let item = event["item_id"].as_str().unwrap_or_default();
                match event["type"].as_str() {
                    Some("conversation.item.input_audio_transcription.delta") => {
                        let segment = items.segment(item, speaking.current.as_ref());
                        if let (Some(segment), Some(text)) = (segment, event["delta"].as_str()) {
                            let _ = deltas.send(Delta { segment, text: text.to_owned() }).await;
                        }
                    }
                    Some("input_audio_buffer.committed") => items.committed(item.to_owned()),
                    Some("conversation.item.input_audio_transcription.completed") => {
                        items.completed(item);
                    }
                    // 1 つの要求の失敗ではセッションは続く。commit が断られたら、その音声を消して次に混ぜない
                    Some("error") => {
                        tracing::warn!("realtime error: {}", event["error"]);
                        if let Some(segment) = event["error"]["event_id"]
                            .as_str()
                            .and_then(|id| id.strip_prefix("commit-"))
                        {
                            items.drop_segment(segment);
                            sink.send(clear()).await?;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type Sink = futures_util::stream::SplitSink<Socket, Message>;
type Stream = futures_util::stream::SplitStream<Socket>;

async fn connect(cfg: &RealtimeConfig, hints: &Hints) -> anyhow::Result<(Sink, Stream)> {
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
    Ok((sink, stream))
}

/// 今の発話を閉じる。短すぎれば commit せず、サーバーに溜まった音声を消す（次の発話に混ぜない）
async fn close(
    sink: &mut Sink,
    speaking: &mut Speaking,
    items: &mut Items,
    pcm: &mut Vec<u8>,
    buffered: usize,
) -> anyhow::Result<()> {
    let Some(segment) = speaking.current.take() else {
        return Ok(());
    };
    if !pcm.is_empty() {
        sink.send(append(pcm)).await?;
    }
    if buffered >= MIN_COMMIT_SAMPLES {
        items.committing.push_back(segment.clone());
        let commit =
            json!({ "type": "input_audio_buffer.commit", "event_id": format!("commit-{segment}") });
        sink.send(Message::text(commit.to_string())).await?;
    } else {
        items.drop_segment(&segment);
        sink.send(clear()).await?;
    }
    Ok(())
}

fn clear() -> Message {
    Message::text(json!({ "type": "input_audio_buffer.clear" }).to_string())
}

/// 16kHz を 24kHz にして PCM に足し、足したサンプル数を返す
fn push(upsampler: &mut Upsampler, pcm: &mut Vec<u8>, chunk: &[f32]) -> usize {
    let samples = upsampler.process(chunk);
    pcm.extend(samples.iter().flat_map(|s| to_i16(*s).to_le_bytes()));
    samples.len()
}

/// 16kHz → 24kHz の線形補間。ため込まないので、発話の終わりまでその場で送れる
#[derive(Default)]
struct Upsampler {
    /// 直前のチャンクの最後のサンプル
    prev: Option<f32>,
    /// 次に出すサンプルの位置（`prev` を 0 とした入力のサンプル単位）
    pos: f64,
}

impl Upsampler {
    const STEP: f64 = SAMPLE_RATE as f64 / REALTIME_RATE as f64;

    fn process(&mut self, chunk: &[f32]) -> Vec<f32> {
        let Some(&first) = chunk.first() else {
            return Vec::new();
        };
        // 最初のチャンクは、前のサンプルがないので入力の先頭（位置 1）から始める
        let prev = match self.prev {
            Some(prev) => prev,
            None => {
                self.pos = 1.0;
                first
            }
        };
        let at = |i: usize| if i == 0 { prev } else { chunk[i - 1] };
        let len = chunk.len() + 1;
        let mut out = Vec::with_capacity(chunk.len() * 3 / 2 + 1);
        while self.pos + 1.0 < len as f64 {
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            out.push(at(i) + (at(i + 1) - at(i)) * frac);
            self.pos += Self::STEP;
        }
        self.pos -= (len - 1) as f64;
        self.prev = chunk.last().copied();
        out
    }
}

/// 溜めた PCM を input_audio_buffer.append にして空にする
fn append(pcm: &mut Vec<u8>) -> Message {
    let audio = base64::engine::general_purpose::STANDARD.encode(&*pcm);
    pcm.clear();
    Message::text(json!({ "type": "input_audio_buffer.append", "audio": audio }).to_string())
}

async fn expect_event(stream: &mut Stream, expected: &str) -> anyhow::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsampler_makes_three_samples_from_two_without_holding_any() {
        let input: Vec<f32> = (0..1600).map(|i| i as f32).collect();
        // チャンクに分けても、まとめても同じ数・同じ値になる
        let whole = Upsampler::default().process(&input);
        let mut up = Upsampler::default();
        let parts: Vec<f32> = input.chunks(321).flat_map(|c| up.process(c)).collect();
        assert_eq!(whole.len(), parts.len());
        assert!(whole.iter().zip(&parts).all(|(a, b)| (a - b).abs() < 1e-3));
        assert!((whole.len() as i64 - 2400).abs() <= 2);
        // 線形の入力は線形のまま（0, 2/3, 4/3, ...）
        assert!((whole[3] - 2.0).abs() < 1e-3);
    }

    #[test]
    fn items_follow_commit_order() {
        let mut items = Items::default();
        let (a, b, c) = ("a".to_owned(), "b".to_owned(), "c".to_owned());
        // 話している途中の delta は今の発話へ
        assert_eq!(items.segment("i1", Some(&a)), Some(a.clone()));
        items.committing.push_back(a.clone());
        items.committed("i1".into());
        // A の item に delta がなく、確認の前に B の item の delta が来ると、いったん A と見なすが、
        // 確認で B に付け直す
        items.committing.push_back(a.clone());
        items.committing.push_back(b.clone());
        assert_eq!(items.segment("i3", None), Some(a.clone()));
        items.committed("i2".into());
        items.committed("i3".into());
        assert_eq!(items.segment("i3", Some(&c)), Some(b.clone()));
        // 文字が届き終わった item と、捨てた発話の対応は消える
        items.completed("i3");
        assert_eq!(items.segment("i9", Some(&c)), Some(c.clone()));
        items.drop_segment("c");
        assert!(!items.by_item.values().any(|s| s == "c"));
    }

    #[test]
    fn preroll_holds_only_audio_between_utterances() {
        let mut speaking = Speaking::default();
        speaking.skip(Input::Audio(vec![0.1; 100]));
        speaking.skip(Input::Open("a".into()));
        speaking.skip(Input::Audio(vec![0.2; 100]));
        assert_eq!(speaking.preroll.len(), 100);
        speaking.skip(Input::Close);
        assert!(speaking.preroll.is_empty());
    }
}
