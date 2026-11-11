//! Qwen-Audio-3.1-ASR-Flash（QwenCloud シンガポール）。OpenAI 互換の口では使えず、独自の口で送る（DESIGN.md 3.4）。
//!
//! 確定は 1 回の HTTP（`/api/v1/services/aigc/multimodal-generation/generation`）。
//! 途中の字幕は WebSocket（`/api-ws/v1/inference`）に 16kHz の PCM を流す。文の区切りはサーバーが決めるので、
//! 文に付いてくる時刻（流した音声の先頭からのミリ秒）で、アプリの発話に割り当てる

use std::collections::BTreeMap;

use tokio::time::Instant;

use super::*;

/// 確定に添える用語は、前の発話の文として渡す。400 文字まで（越えた分は切られる）
const CONTEXT_CHARS: usize = 400;

/// 発話を閉じたあとに流す無音。サーバーが文を終わらせ、次の発話とつなげない
const SILENCE_AFTER: usize = SAMPLE_RATE as usize * 6 / 10;
/// 音声を 23 秒送らないと切られるので、黙っている間は無音を少しだけ流す（マイクの音は送らない）
const KEEP_ALIVE: Duration = Duration::from_secs(10);
const KEEP_ALIVE_SAMPLES: usize = SAMPLE_RATE as usize / 10;
/// 文を割り当てるために覚えておく発話の数
const MAX_SPANS: usize = 16;
/// 文の始まりが発話を開いた位置より少し前でも、その発話のものとみなす（時刻は 1 文字ごとに付き、少しずれる）
const EARLY: usize = SAMPLE_RATE as usize * 2 / 10;

/// 送り先の根元（`https://…`）。鍵の送り先は OpenAI 互換の口なので、その手前まで
fn root(access: &Access) -> String {
    let base = access.url("");
    base.strip_suffix("/compatible-mode/v1")
        .unwrap_or(&base)
        .to_owned()
}

/// 発話区間の WAV を文字起こしする
pub(super) async fn transcribe(
    http: &reqwest::Client,
    access: &Access,
    req: BatchRequest<'_>,
) -> anyhow::Result<String> {
    let mut content = Vec::new();
    let mut context = String::new();
    for keyword in &req.hints.keywords {
        if context.chars().count() + keyword.chars().count() + 1 > CONTEXT_CHARS {
            break;
        }
        context += keyword;
        context += "、";
    }
    if !context.is_empty() {
        content.push(json!({ "type": "input_text", "text": context }));
    }
    let audio = base64::engine::general_purpose::STANDARD.encode(&req.wav);
    content.push(json!({
        "type": "input_audio",
        "input_audio": { "data": format!("data:audio/wav;base64,{audio}") },
    }));
    let body = json!({
        "model": req.model,
        "input": { "messages": [{ "role": "user", "content": content }] },
        "parameters": {
            "format": "wav",
            "sample_rate": SAMPLE_RATE.to_string(),
            "language_hints": [req.hints.language],
        },
    });
    let url = format!(
        "{}/api/v1/services/aigc/multimodal-generation/generation",
        root(access)
    );
    let res = http
        .post(url)
        .bearer_auth(access.token())
        .header("X-DashScope-SSE", "disable")
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        if let Some(refused) = share::refusal(access, status, None) {
            return Err(refused.into());
        }
        bail!("{status}: {body}");
    }
    let res: Value = res.json().await?;
    req.meter
        .tokens(Use::Batch, req.model, Tokens::from_response(&res["usage"]));
    let output = &res["output"];
    let text = output["text"]
        .as_str()
        .or_else(|| output["sentence"]["text"].as_str())
        .or_else(|| output["choices"][0]["message"]["content"][0]["text"].as_str());
    text.map(|t| t.trim().to_owned())
        .context("transcription response has no text")
}

/// 1 つの接続の中の、流した音声と文の対応
#[derive(Default)]
struct Session {
    /// 流したサンプル数（無音も含む）
    sent: usize,
    /// 発話を開いたときの `sent` と、その発話。古い順
    spans: VecDeque<(usize, String)>,
    /// 文の番号 → 発話ごとに分けた、今の文字（発話の順）
    sentences: BTreeMap<u64, Vec<(String, String)>>,
    /// 最後に届いた使用量（接続を始めてからの合計で届く）
    usage: (u64, u64),
}

impl Session {
    fn open(&mut self, segment: String) {
        self.spans.push_back((self.sent, segment));
        if self.spans.len() > MAX_SPANS {
            self.spans.pop_front();
            let spans = &self.spans;
            self.sentences.retain(|_, parts| {
                let known =
                    |(segment, _): &(String, String)| spans.iter().any(|(_, s)| s == segment);
                parts.iter().any(known)
            });
        }
    }

    /// その時刻（ミリ秒）に開いていた発話。どの発話より前なら、いちばん古い発話
    fn segment_at(&self, ms: u64) -> Option<&String> {
        let at = ms as usize * SAMPLE_RATE as usize / 1000;
        let opened = self.spans.iter().rev();
        let mut opened = opened.skip_while(|(start, _)| *start > at + EARLY);
        let (_, segment) = opened.next().or_else(|| self.spans.front())?;
        Some(segment)
    }

    /// 届いた文を発話に割り当て、変わった発話ごとに、今の文字（文をつなげたもの）を返す。
    /// 文は書きかけのうちから何度も届き、届くたびに文字が置き換わる。サーバーの文はアプリの発話をまたぐ
    /// ことがある（発話の終わりで文が終わるとは限らない）ので、文字ごとの時刻で分ける
    fn on_sentence(&mut self, sentence: &Value) -> Vec<Delta> {
        let Some(id) = sentence["sentence_id"].as_u64() else {
            return Vec::new();
        };
        let mut parts: Vec<(String, String)> = Vec::new();
        let mut add = |segment: Option<&String>, text: &str| {
            let Some(segment) = segment else { return };
            match parts.last_mut() {
                Some((last, said)) if last == segment => *said += text,
                _ => parts.push((segment.clone(), text.to_owned())),
            }
        };
        let words = sentence["words"].as_array().filter(|w| !w.is_empty());
        match words {
            Some(words) => {
                for word in words {
                    let segment = word["begin_time"]
                        .as_u64()
                        .and_then(|ms| self.segment_at(ms));
                    add(segment, word["text"].as_str().unwrap_or_default());
                    add(segment, word["punctuation"].as_str().unwrap_or_default());
                }
            }
            // 文字ごとの時刻がなければ、文の始まりの時刻で決める
            None => {
                let begin = sentence["begin_time"].as_u64();
                let text = sentence["text"].as_str().unwrap_or_default();
                add(begin.and_then(|ms| self.segment_at(ms)), text);
            }
        }
        parts.retain(|(_, text)| !text.is_empty());
        let before = self.sentences.insert(id, parts.clone()).unwrap_or_default();
        // 前はあったのに消えた発話（文字が直された）も、置き換えを知らせる
        let mut changed: Vec<String> = Vec::new();
        for (segment, _) in parts.iter().chain(&before) {
            if !changed.contains(segment) {
                changed.push(segment.clone());
            }
        }
        let deltas = changed.into_iter().map(|segment| {
            let said = self.sentences.values().flatten();
            let text = said
                .filter(|(s, _)| *s == segment)
                .map(|(_, text)| text.as_str())
                .collect();
            Delta {
                segment,
                text,
                whole: true,
            }
        });
        deltas.collect()
    }

    /// 使用量の増えた分
    fn spent(&mut self, usage: &Value) -> Option<Tokens> {
        let now = (
            usage["input_tokens"].as_u64()?,
            usage["output_tokens"].as_u64()?,
        );
        let before = std::mem::replace(&mut self.usage, now);
        Some(Tokens {
            input: now.0.saturating_sub(before.0),
            cached: 0,
            output: now.1.saturating_sub(before.1),
        })
    }
}

/// 1 セッション分。入力が尽きたら Ok、切れたら Err で返る（`super::realtime` と同じ決まり）
pub(super) async fn live(
    cfg: &RealtimeConfig,
    hints: &Hints,
    speaking: &mut Speaking,
    input: &mut mpsc::UnboundedReceiver<Input>,
    deltas: &mpsc::Sender<Delta>,
) -> anyhow::Result<()> {
    let task = format!("{:032x}", rand_id());
    let header = |action: &str| json!({ "action": action, "task_id": task, "streaming": "duplex" });
    let (mut sink, mut stream) =
        tokio::time::timeout(HANDSHAKE_TIMEOUT, connect(cfg, hints, header("run-task")))
            .await
            .context("realtime handshake timed out")??;
    tracing::info!("realtime session started");

    // 接続している間に溜まった入力は送らない（古い言葉が今の発話に混ざる）。今の発話の続きから送る
    while let Ok(next) = input.try_recv() {
        speaking.skip(next);
    }
    let mut session = Session::default();
    let mut last_sent = Instant::now();
    macro_rules! send {
        ($samples:expr) => {{
            let samples: &[f32] = $samples;
            let pcm: Vec<u8> = samples
                .iter()
                .flat_map(|s| to_i16(*s).to_le_bytes())
                .collect();
            session.sent += samples.len();
            last_sent = Instant::now();
            sink.send(Message::binary(pcm)).await?;
        }};
    }
    if let Some(id) = speaking.current.clone() {
        session.open(id);
        let preroll: Vec<f32> = speaking.preroll.drain(..).collect();
        send!(&preroll);
    }
    loop {
        tokio::select! {
            next = input.recv() => match next {
                Some(Input::Open(id)) => {
                    speaking.current = Some(id.clone());
                    session.open(id);
                    let preroll: Vec<f32> = speaking.preroll.drain(..).collect();
                    send!(&preroll);
                }
                Some(Input::Audio(chunk)) => {
                    if speaking.current.is_some() {
                        send!(&chunk);
                    } else {
                        speaking.remember(&chunk);
                    }
                }
                Some(Input::Close) => {
                    if speaking.current.take().is_some() {
                        send!(&[0.0; SILENCE_AFTER]);
                    }
                }
                None => {
                    let finish = json!({ "header": header("finish-task"), "payload": { "input": {} } });
                    let _ = sink.send(Message::text(finish.to_string())).await;
                    return Ok(());
                }
            },
            () = tokio::time::sleep_until(last_sent + KEEP_ALIVE) => send!(&[0.0; KEEP_ALIVE_SAMPLES]),
            msg = stream.next() => {
                let event = match msg {
                    Some(Ok(Message::Text(text))) => serde_json::from_str::<Value>(&text)?,
                    Some(Ok(Message::Close(frame))) => bail!("realtime closed: {frame:?}"),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(e.into()),
                    None => bail!("realtime stream ended"),
                };
                match event["header"]["event"].as_str() {
                    Some("result-generated") => {
                        if let Some(tokens) = session.spent(&event["payload"]["usage"]) {
                            cfg.meter.tokens(Use::Realtime, &cfg.model, tokens);
                        }
                        for delta in session.on_sentence(&event["payload"]["output"]["sentence"]) {
                            let _ = deltas.send(delta).await;
                        }
                    }
                    Some("task-failed") => return Err(failed(cfg, &event["header"])),
                    Some("task-finished") => bail!("realtime task finished"),
                    _ => {}
                }
            }
        }
    }
}

/// 接続の名前（重ならなければよい）
fn rand_id() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    nanos ^ (u128::from(std::process::id()) << 96)
}

/// サーバーが断った理由。鍵が使えないなら、張り直しても同じなので字幕を止める
fn failed(cfg: &RealtimeConfig, header: &Value) -> anyhow::Error {
    let code = header["error_code"].as_str().unwrap_or_default();
    if code.contains("InvalidApiKey") || code.contains("Unauthorized") {
        let provider = cfg.access.provider();
        return TranscriptionError::InvalidKey { provider }.into();
    }
    anyhow::anyhow!("realtime task failed: {header}")
}

async fn connect(
    cfg: &RealtimeConfig,
    hints: &Hints,
    run: Value,
) -> anyhow::Result<(Sink, Stream)> {
    let url = root(&cfg.access)
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let mut request = format!("{url}/api-ws/v1/inference").into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("bearer {}", cfg.access.token()))?,
    );
    let (ws, _) = match tokio_tungstenite::connect_async(request).await {
        Ok(connected) => connected,
        Err(e) => {
            if let tungstenite::Error::Http(res) = &e
                && let Some(refused) = share::refusal(&cfg.access, res.status(), None)
            {
                return Err(refused.into());
            }
            return Err(e).context("connect realtime");
        }
    };
    let (mut sink, mut stream) = ws.split();
    let start = json!({
        "header": run,
        "payload": {
            "task_group": "audio", "task": "asr", "function": "recognition", "model": cfg.model,
            "parameters": {
                "format": "pcm",
                "sample_rate": SAMPLE_RATE,
                "language_hints": [hints.language],
            },
            "input": {},
        },
    });
    sink.send(Message::text(start.to_string())).await?;
    while let Some(msg) = stream.next().await {
        let Message::Text(text) = msg? else { continue };
        let event: Value = serde_json::from_str(&text)?;
        match event["header"]["event"].as_str() {
            Some("task-started") => return Ok((sink, stream)),
            Some("task-failed") => return Err(failed(cfg, &event["header"])),
            _ => {}
        }
    }
    bail!("realtime closed before task-started")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 文字ごとの時刻（ミリ秒）と文字
    fn sentence(id: u64, words: &[(u64, &str)]) -> Value {
        let words: Vec<Value> = words
            .iter()
            .map(|(ms, text)| json!({ "begin_time": ms, "text": text, "punctuation": "" }))
            .collect();
        json!({ "sentence_id": id, "begin_time": 0, "text": "", "words": words })
    }

    fn said(session: &mut Session, sentence: Value) -> Vec<(String, String)> {
        let deltas = session.on_sentence(&sentence).into_iter();
        deltas.map(|d| (d.segment, d.text)).collect()
    }

    fn of(segment: &str, text: &str) -> (String, String) {
        (segment.to_owned(), text.to_owned())
    }

    #[test]
    fn words_go_to_the_utterance_that_was_open_when_they_were_said() {
        let mut session = Session::default();
        session.open("a".into());
        session.sent += 3 * SAMPLE_RATE as usize;
        session.open("b".into());
        // 書きかけの文は、届くたびに置き換わる
        assert_eq!(
            said(&mut session, sentence(1, &[(400, "さあ")])),
            [of("a", "さあ")]
        );
        let first = sentence(1, &[(400, "さあ"), (900, "始まりました。")]);
        assert_eq!(said(&mut session, first), [of("a", "さあ始まりました。")]);
        // 同じ発話の 2 つ目の文は後ろにつなげる。発話をまたいだ文は、文字の時刻で分ける
        // （次の発話を開いた位置より少し前の文字も、次の発話のもの）
        let second = sentence(2, &[(1800, "実況です。"), (2900, "ヘイズ")]);
        assert_eq!(
            said(&mut session, second),
            [of("a", "さあ始まりました。実況です。"), of("b", "ヘイズ")]
        );
        // 文字が直されて発話から消えたら、その発話も置き換える
        let fixed = sentence(2, &[(1800, "実況です。")]);
        assert_eq!(
            said(&mut session, fixed),
            [of("a", "さあ始まりました。実況です。"), of("b", "")]
        );
        // 文字のない知らせ（文の始まり）は何も出さない
        assert!(said(&mut session, sentence(3, &[])).is_empty());
    }

    #[test]
    fn a_sentence_without_word_times_goes_by_its_beginning() {
        let mut session = Session::default();
        session.open("a".into());
        let whole = json!({ "sentence_id": 1, "begin_time": 500, "text": "さあ" });
        assert_eq!(said(&mut session, whole), [of("a", "さあ")]);
    }

    #[test]
    fn usage_arrives_as_a_running_total() {
        let mut session = Session::default();
        let usage =
            |input: u64, output: u64| json!({ "input_tokens": input, "output_tokens": output });
        let spent = session.spent(&usage(57, 10)).unwrap();
        assert_eq!((spent.input, spent.output), (57, 10));
        let spent = session.spent(&usage(120, 25)).unwrap();
        assert_eq!((spent.input, spent.output), (63, 15));
        assert!(session.spent(&Value::Null).is_none());
    }
}
