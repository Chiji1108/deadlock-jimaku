//! 音声認識（DESIGN.md 3 章）。途中の字幕（リアルタイム）と、確定の文字起こし（バッチ）。
//!
//! ここには、どの会社でも同じもの（手がかり、入力、途中の文字、接続をまたいで持つ状態）と、会社への振り分けを置く。
//! 発話の区切りはアプリが決める（`segment.rs`）。途中の字幕は、その区切りに会社ごとのやり方で合わせる:
//! OpenAI の Realtime は発話ごとに commit し（`openai.rs`）、Qwen の Streaming は文字ごとの時刻で割り当てる（`qwen.rs`）。
//! 確定は、発話の WAV を 1 回送る（OpenAI、xAI、OpenRouter、Qwen、PC の中の speech.cpp）

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::{self, Message, client::IntoClientRequest, http::HeaderValue};

use crate::access::Access;
use crate::audio::{SAMPLE_RATE, to_i16};
use crate::local::Speech;
use crate::secrets::Provider;
use crate::share;
use crate::types::TranscriptionError;
use crate::usage::{Meter, Tokens, Use};
use crate::whisper::Whisper;

mod local;
mod openai;
mod qwen;

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
    /// ヒーローの名前（話す言語で）。少ししか用語を渡せない Whisper に渡す（DESIGN.md 9.10）
    pub heroes: Vec<String>,
}

pub struct RealtimeConfig {
    /// 途中の字幕を頼む相手
    pub transcriber: Transcriber,
    pub model: String,
    /// `minimal` / `low` / `medium` / `high` / `xhigh`（低いほど早く、高いほど正確）
    pub delay: String,
    /// 送った音声の長さを数える
    pub meter: Arc<Meter>,
}

/// リアルタイムに送るもの。同じチャンネルで送り、順番を保つ
pub enum Input {
    /// 発話を開いた（Segment の ID）。ここから閉じるまでの音声がこの発話になる
    Open(String),
    Audio(Vec<f32>),
    /// 今の発話を閉じた
    Close,
}

/// 発話（Segment の ID）の途中の文字
pub struct Delta {
    pub segment: String,
    pub text: String,
    /// `text` は発話の今の文字の全部（置き換える）。false なら、続きの文字（追記する）
    pub whole: bool,
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

/// 1 セッション分。入力が尽きたら Ok（最後の発話の確定はバッチで取る）、切れたら Err で返る。
/// 途中の文字を発話に合わせるやり方は会社ごとに違う（`openai::Items`、`qwen::Session`）
pub async fn realtime(
    cfg: &RealtimeConfig,
    hints: &Hints,
    speaking: &mut Speaking,
    input: &mut mpsc::UnboundedReceiver<Input>,
    deltas: &mpsc::Sender<Delta>,
) -> anyhow::Result<()> {
    match &cfg.transcriber {
        Transcriber::Api(access) if access.provider() == Provider::Qwen => {
            qwen::live(cfg, access, hints, speaking, input, deltas).await
        }
        Transcriber::Api(_) => openai::live(cfg, hints, speaking, input, deltas).await,
        Transcriber::Local(speech) => {
            local::live(&local::Recognizer::Speech(speech), speaking, input, deltas).await
        }
        Transcriber::Whisper(whisper) => {
            let recognizer = local::Recognizer::Whisper(whisper, hints);
            local::live(&recognizer, speaking, input, deltas).await
        }
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type Sink = futures_util::stream::SplitSink<Socket, Message>;
type Stream = futures_util::stream::SplitStream<Socket>;

/// xAI に渡せる用語は 100 件、1 件 50 文字まで
const XAI_MAX_KEYTERMS: usize = 100;
const XAI_MAX_KEYTERM_CHARS: usize = 50;

/// xAI の言語コード。アプリの話す言語は、ノルウェー語のほかは同じコードで xAI も受け付ける（2026-11 の一覧）
fn xai_language(code: &str) -> &str {
    match code {
        "no" => "nb",
        code => code,
    }
}

/// xAI が鍵を断った（DESIGN.md 3）。間違った鍵には 401 ではなく 400 と「Incorrect API key provided」を返す（2026-11 の実測）
fn xai_refused(status: StatusCode, body: &str) -> bool {
    status == StatusCode::UNAUTHORIZED
        || (status == StatusCode::BAD_REQUEST && body.contains("Incorrect API key"))
}

/// 文字起こしを頼む相手（途中の字幕と確定で同じ形）
#[derive(Clone)]
pub enum Transcriber {
    /// 会社の API（OpenAI、xAI、Qwen、OpenRouter）
    Api(Access),
    /// PC の中で動く speech.cpp（Haas-ja）。途中の字幕と確定で、同じ 1 つを使う
    Local(Arc<Speech>),
    /// PC の中で動く whisper.cpp。途中の字幕と確定で、同じ 1 つを使う
    Whisper(Arc<Whisper>),
}

impl Transcriber {
    /// その工程を頼む相手。会社（`provider`）があればその送り先と鍵、なければ PC の中
    pub fn of(
        provider: Option<Provider>,
        accesses: &crate::access::Accesses,
        local: &Option<Arc<Speech>>,
    ) -> Option<Self> {
        match provider {
            Some(provider) => accesses.get(provider).map(Self::Api),
            None => local.clone().map(Self::Local),
        }
    }
}

pub struct BatchRequest<'a> {
    pub transcriber: &'a Transcriber,
    pub model: &'a str,
    pub hints: &'a Hints,
    pub wav: Vec<u8>,
    /// WAV の長さ（秒。料金を数える）
    pub seconds: f64,
    pub meter: &'a Meter,
}

/// 発話区間の WAV を文字起こしする
pub async fn transcribe(http: &reqwest::Client, req: BatchRequest<'_>) -> anyhow::Result<String> {
    if let Transcriber::Api(access) = req.transcriber
        && access.provider() == Provider::Qwen
    {
        return qwen::transcribe(http, access, req).await;
    }
    let file = reqwest::multipart::Part::bytes(req.wav)
        .file_name("segment.wav")
        .mime_str("audio/wav")?;
    let access = match req.transcriber {
        Transcriber::Api(access) => access,
        Transcriber::Local(speech) => {
            // 用語は渡さない（辞書の名前を全部渡すと、話していない用語を書いた。DESIGN.md 9.7）
            let form = reqwest::multipart::Form::new().part("file", file);
            let res = speech.transcriptions().multipart(form).send().await?;
            let status = res.status();
            if !status.is_success() {
                bail!("{status}: {}", res.text().await.unwrap_or_default());
            }
            req.meter.audio(Use::Batch, req.model, req.seconds);
            return text(&res.json().await?);
        }
        Transcriber::Whisper(whisper) => {
            let form = whisper_form(req.hints).part("file", file);
            let request = whisper.transcriptions().multipart(form);
            let res = request.send().await?;
            let status = res.status();
            if !status.is_success() {
                bail!("{status}: {}", res.text().await.unwrap_or_default());
            }
            req.meter.audio(Use::Batch, req.model, req.seconds);
            return text(&res.json().await?);
        }
    };
    let form = reqwest::multipart::Form::new().text("model", req.model.to_owned());
    let request = match access.provider() {
        Provider::OpenAi => {
            let mut form = form
                .text("prompt", req.hints.prompt.clone())
                .text("languages[]", req.hints.language.clone());
            for keyword in &req.hints.keywords {
                form = form.text("keywords[]", keyword.clone());
            }
            http.post(access.url("/audio/transcriptions"))
                .bearer_auth(access.token())
                .multipart(form.part("file", file))
        }
        Provider::Xai => {
            // ファイルより後ろの欄は読まれないことがあるので、ファイルは最後に置く
            let mut form = form.text("language", xai_language(&req.hints.language).to_owned());
            let keyterms = req.hints.keywords.iter();
            let keyterms = keyterms.filter(|k| k.chars().count() <= XAI_MAX_KEYTERM_CHARS);
            for keyword in keyterms.take(XAI_MAX_KEYTERMS) {
                form = form.text("keyterm", keyword.clone());
            }
            http.post(access.url("/stt"))
                .bearer_auth(access.token())
                .multipart(form.part("file", file))
        }
        // 用語を渡す欄はない
        Provider::OpenRouter => {
            let form = form.text("language", req.hints.language.clone());
            http.post(access.url("/audio/transcriptions"))
                .bearer_auth(access.token())
                .multipart(form.part("file", file))
        }
        // Qwen は上で送っている
        Provider::Qwen | Provider::TypeSafe => bail!("{:?} does not transcribe", access.provider()),
    };
    let res = request.send().await?;
    let status = res.status();
    if !status.is_success() {
        // 400 の理由は本文にしか書かれていない
        let body = res.text().await.unwrap_or_default();
        // xAI の断り方を OpenAI と同じ 401 にそろえる
        let status = match access.provider() {
            Provider::Xai if xai_refused(status, &body) => StatusCode::UNAUTHORIZED,
            _ => status,
        };
        let reason = share::refusal_reason(&body);
        if let Some(refused) = share::refusal(access, status, reason.as_deref()) {
            return Err(refused.into());
        }
        bail!("{status}: {body}");
    }
    req.meter.audio(Use::Batch, req.model, req.seconds);
    text(&res.json().await?)
}

/// whisper.cpp に送る欄（音声のほか）。言語と、ヒーローの名前。
/// 渡せる用語は少しだけ（全部渡すと効かず、遅くなる）。ヒーローの名前で、用語がよく書けるようになる
fn whisper_form(hints: &Hints) -> reqwest::multipart::Form {
    let form = reqwest::multipart::Form::new()
        .text("language", hints.language.clone())
        .text("response_format", "json");
    if hints.heroes.is_empty() {
        return form;
    }
    let comma = if hints.language == "ja" { "、" } else { ", " };
    form.text("prompt", hints.heroes.join(comma))
}

fn text(res: &Value) -> anyhow::Result<String> {
    res["text"]
        .as_str()
        .map(|t| t.trim().to_owned())
        .context("transcription response has no text")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本物の whisper.cpp とモデルを取って（約 600 MB）、合成音声を文字起こしする。ヒーローの名前を渡す。
    /// ふだんは動かさない: `WHISPER_TEST_DIR=… cargo test -p overlays-core whisper_writes -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "downloads whisper.cpp and the model"]
    async fn whisper_writes_the_heroes_it_was_told() {
        let dir = std::env::var("WHISPER_TEST_DIR").expect("WHISPER_TEST_DIR");
        let size = crate::whisper::Size::Turbo;
        let whisper = Whisper::start(std::path::Path::new(&dir), size, &|_, _| {}).await;
        let wav = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../experiments/data/tts/aya/001.wav"
        );
        let meter = Meter::new(std::env::temp_dir().join("whisper_test_usage.json"));
        let request = BatchRequest {
            transcriber: &Transcriber::Whisper(Arc::new(whisper.unwrap())),
            model: size.name(),
            hints: &Hints {
                language: "ja".into(),
                prompt: String::new(),
                keywords: Vec::new(),
                heroes: vec!["ヘイズ".into(), "インファーナス".into()],
            },
            wav: std::fs::read(wav).unwrap(),
            seconds: 4.0,
            meter: &meter,
        };
        let text = transcribe(&reqwest::Client::new(), request).await.unwrap();
        println!("{text}");
        assert!(text.contains("ヘイズ"), "{text}");
    }

    /// 本物の speech.cpp とモデルを取って（約 400 MB）、録音を文字起こしする。ふだんは動かさない:
    /// `cargo test -p overlays-core haas_ja -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "downloads speech.cpp and the model"]
    async fn haas_ja_transcribes_a_recording_on_this_computer() {
        let dir = std::env::var("HAAS_JA_TEST_DIR").map_or_else(
            |_| std::env::temp_dir().join("overlays-speech-test"),
            std::path::PathBuf::from,
        );
        let started = std::time::Instant::now();
        let speech = Speech::start(&dir).await.unwrap();
        println!("ready in {:?}", started.elapsed());
        let wav = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../experiments/data/sample01.wav"
        );
        let meter = Meter::new(dir.join("usage.json"));
        let started = std::time::Instant::now();
        let request = BatchRequest {
            transcriber: &Transcriber::Local(Arc::new(speech)),
            model: crate::local::HAAS_JA,
            hints: &Hints {
                language: "ja".into(),
                prompt: String::new(),
                keywords: Vec::new(),
                heroes: Vec::new(),
            },
            wav: std::fs::read(wav).unwrap(),
            seconds: 21.8,
            meter: &meter,
        };
        let text = transcribe(crate::http(), request).await.unwrap();
        println!("{:?}: {text}", started.elapsed());
        assert!(text.contains("デッドロック"));
        let batch = meter.watch().borrow().session[&Use::Batch].clone();
        assert!(batch.seconds > 0.0 && batch.usd == 0.0 && !batch.unpriced);
    }

    /// 本物の OpenRouter に録音を送る（`OPENROUTER_API_KEY`）。ふだんは動かさない:
    /// `cargo test -p overlays-core mai_transcribe -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "calls OpenRouter"]
    async fn mai_transcribe_2_transcribes_through_openrouter() {
        let key = std::env::var("OPENROUTER_API_KEY").unwrap();
        let wav = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../experiments/data/sample01.wav"
        );
        let meter = Meter::new(std::env::temp_dir().join("overlays-mai-test-usage.json"));
        let request = BatchRequest {
            transcriber: &Transcriber::Api(Access::key(Provider::OpenRouter, key)),
            model: "microsoft/mai-transcribe-2",
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
        let text = transcribe(crate::http(), request).await.unwrap();
        println!("{text}");
        assert!(text.contains("デッドロック"));
        let batch = meter.watch().borrow().session[&Use::Batch].clone();
        assert!(batch.usd > 0.0 && !batch.unpriced);
    }

    fn sample(name: &str) -> String {
        format!(
            "{}/../../experiments/data/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    /// 本物の QwenCloud に録音を送る（`QWEN_SOUTH_API_KEY`）。ふだんは動かさない:
    /// `cargo test -p overlays-core qwen_audio -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "calls QwenCloud"]
    async fn qwen_audio_transcribes_a_recording() {
        let key = std::env::var("QWEN_SOUTH_API_KEY").unwrap();
        let meter = Meter::new(std::env::temp_dir().join("overlays-qwen-test-usage.json"));
        let started = std::time::Instant::now();
        let request = BatchRequest {
            transcriber: &Transcriber::Api(Access::key(Provider::Qwen, key)),
            model: "qwen-audio-3.1-asr-flash",
            hints: &Hints {
                language: "ja".into(),
                prompt: String::new(),
                keywords: vec!["ヘイズ".into(), "バイオレット".into()],
                heroes: Vec::new(),
            },
            wav: std::fs::read(sample("sample01.wav")).unwrap(),
            seconds: 21.8,
            meter: &meter,
        };
        let text = transcribe(crate::http(), request).await.unwrap();
        println!("{:?}: {text}", started.elapsed());
        assert!(text.contains("デッドロック"));
        let batch = meter.watch().borrow().session[&Use::Batch].clone();
        println!("{batch:?}");
        assert!(batch.usd > 0.0 && !batch.unpriced);
    }

    /// 録音の一部（`spans`。名前、始まりと終わりのミリ秒、そのあと黙る秒数）を、それぞれ 1 つの発話として実時間で流し、
    /// 途中の文字がどの発話に届くかを出す
    async fn said_live(
        cfg: &RealtimeConfig,
        hints: &Hints,
        spans: &'static [(&'static str, usize, usize, u64)],
    ) -> std::collections::BTreeMap<String, String> {
        let samples: Vec<f32> = hound::WavReader::open(sample("sample01.wav"))
            .unwrap()
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / f32::from(i16::MAX))
            .collect();
        let (input_tx, mut input) = mpsc::unbounded_channel();
        let (deltas_tx, mut deltas) = mpsc::channel(256);
        let started = std::time::Instant::now();
        let feed = tokio::spawn(async move {
            let at = |ms: usize| (ms * SAMPLE_RATE as usize / 1000).min(samples.len());
            for &(id, start, end, pause) in spans {
                let part = &samples[at(start)..at(end)];
                input_tx.send(Input::Open(id.into())).unwrap();
                for chunk in part.chunks(SAMPLE_RATE as usize / 10) {
                    input_tx.send(Input::Audio(chunk.to_vec())).unwrap();
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                input_tx.send(Input::Close).unwrap();
                println!("{:5.1}s {id} closed", started.elapsed().as_secs_f32());
                tokio::time::sleep(Duration::from_secs(pause)).await;
            }
        });
        let mut speaking = Speaking::default();
        let session = realtime(cfg, hints, &mut speaking, &mut input, &deltas_tx);
        let mut last = std::collections::BTreeMap::<String, String>::new();
        let print = async {
            while let Some(delta) = deltas.recv().await {
                let text = last.entry(delta.segment.clone()).or_default();
                if delta.whole {
                    *text = delta.text;
                } else {
                    *text += &delta.text;
                }
                println!(
                    "{:5.1}s {} {text}",
                    started.elapsed().as_secs_f32(),
                    delta.segment
                );
            }
        };
        tokio::select! {
            ended = session => ended.unwrap(),
            () = print => {}
        }
        feed.await.unwrap();
        println!("{last:?}");
        last
    }

    /// 本物の speech.cpp とモデルで、話している間の文字が発話ごとに届くかを見る:
    /// `cargo test -p overlays-core haas_ja_writes -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "downloads speech.cpp and the model"]
    async fn haas_ja_writes_each_utterance_as_it_is_said() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let dir = std::env::var("HAAS_JA_TEST_DIR").map_or_else(
            |_| std::env::temp_dir().join("overlays-speech-test"),
            std::path::PathBuf::from,
        );
        let meter = Arc::new(Meter::new(dir.join("usage-live.json")));
        let cfg = RealtimeConfig {
            transcriber: Transcriber::Local(Arc::new(Speech::start(&dir).await.unwrap())),
            model: crate::local::HAAS_JA.into(),
            delay: "low".into(),
            meter: meter.clone(),
        };
        let hints = Hints {
            language: "ja".into(),
            prompt: "x".into(),
            keywords: vec!["ヘイズ".into()],
            heroes: Vec::new(),
        };
        // アプリの区切り（無音 0.75 秒）で分けた 3 つの発話
        let spans = &[
            ("a", 1392, 4000, 1),
            ("b", 5216, 12336, 1),
            ("c", 13312, 20320, 3),
        ];
        let last = said_live(&cfg, &hints, spans).await;
        assert!(last["a"].contains("デッドロック") && last["c"].contains("バイオレット"));
        // PC の中なので、料金は数えない
        assert!(!meter.watch().borrow().session.contains_key(&Use::Realtime));
    }

    /// 本物の QwenCloud に、録音を 2 つの発話として実時間で流し、途中の文字がどの発話に届くかを見る:
    /// `cargo test -p overlays-core qwen_streaming -- --ignored --nocapture`
    /// `BETA_TEST_API`（開発者のサーバーの根元。`http://127.0.0.1:8799/v1` など）があれば、ベータのコードをもらって中継を通す
    #[tokio::test]
    #[ignore = "calls QwenCloud"]
    async fn qwen_streaming_writes_each_utterance_as_it_is_said() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let access = match std::env::var("BETA_TEST_API") {
            Ok(api) => {
                // コードは 1 日に同じ所から 3 つまでなので、持っていれば `BETA_TEST_CODE` で渡す
                let code = match std::env::var("BETA_TEST_CODE") {
                    Ok(code) => code,
                    Err(_) => crate::share::join_beta(&api).await.unwrap(),
                };
                Access::gift(Provider::Qwen, code, &api)
            }
            Err(_) => Access::key(Provider::Qwen, std::env::var("QWEN_SOUTH_API_KEY").unwrap()),
        };
        let meter = Arc::new(Meter::new(
            std::env::temp_dir().join("overlays-qwen-live-test-usage.json"),
        ));
        let cfg = RealtimeConfig {
            transcriber: Transcriber::Api(access),
            model: "qwen-audio-3.1-asr-flash-streaming".into(),
            delay: "low".into(),
            meter: meter.clone(),
        };
        let hints = Hints {
            language: "ja".into(),
            prompt: String::new(),
            keywords: Vec::new(),
            heroes: Vec::new(),
        };
        // 4.3 秒まで（最初の文の終わり）を 1 つ目、30 秒黙って（切られないことも見る）、残りを 2 つ目の発話にする
        let spans = &[("a", 0, 4300, 30), ("b", 4300, 60_000, 4)];
        let last = said_live(&cfg, &hints, spans).await;
        assert!(last["a"].contains("デッドロック") && last["b"].contains("バイオレット"));
        let live = meter.watch().borrow().session[&Use::Realtime].clone();
        println!("{live:?}");
        assert!(live.usd > 0.0 && !live.unpriced);
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
