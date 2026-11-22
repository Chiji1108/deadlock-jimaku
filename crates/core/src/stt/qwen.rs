//! Qwen-Audio-3.1-ASR-Flash（QwenCloud シンガポール）。OpenAI 互換の口では使えず、独自の口で送る（DESIGN.md 3.4）。
//!
//! 確定は 1 回の HTTP（`/api/v1/services/aigc/multimodal-generation/generation`）。
//! 途中の字幕は WebSocket（`/api-ws/v1/inference`）に 16kHz の PCM を流す。文の区切りは Qwen が決めるので、
//! 文に付いてくる時刻で、アプリの発話に割り当てる（`timed.rs`）

use super::*;

/// 確定に添える用語は、前の発話の文として渡す。400 文字まで（越えた分は切られる）
const CONTEXT_CHARS: usize = 400;

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
        if let Some(refused) = relay::refusal(access, status) {
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

/// Qwen の文を、アプリの形にする。句読点は、その文字の後ろにつなげる
fn sentence(sentence: &Value) -> Option<timed::Sentence> {
    let words = sentence["words"].as_array().into_iter().flatten();
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    Some(timed::Sentence {
        id: sentence["sentence_id"].as_u64()?,
        begin: sentence["begin_time"].as_u64(),
        text: text(&sentence["text"]),
        words: words
            .map(|word| timed::Word {
                at: word["begin_time"].as_u64(),
                text: text(&word["text"]) + word["punctuation"].as_str().unwrap_or_default(),
            })
            .collect(),
    })
}

/// 使用量の増えた分。使用量は、接続を始めてからの合計で届く
fn spent(total: &mut (u64, u64), usage: &Value) -> Option<Tokens> {
    let now = (
        usage["input_tokens"].as_u64()?,
        usage["output_tokens"].as_u64()?,
    );
    let before = std::mem::replace(total, now);
    Some(Tokens {
        input: now.0.saturating_sub(before.0),
        cached: 0,
        output: now.1.saturating_sub(before.1),
    })
}

/// 1 セッション分。入力が尽きたら Ok、切れたら Err で返る（`super::realtime` と同じ決まり）
pub(super) async fn live(
    cfg: &RealtimeConfig,
    access: &Access,
    hints: &Hints,
    speaking: &mut Speaking,
    input: &mut mpsc::UnboundedReceiver<Input>,
    deltas: &mpsc::Sender<Delta>,
) -> anyhow::Result<()> {
    let task = format!("{:032x}", rand_id());
    let header = |action: &str| json!({ "action": action, "task_id": task, "streaming": "duplex" });
    let socket = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        connect(cfg, access, hints, header("run-task")),
    )
    .await
    .context("realtime handshake timed out")??;
    let finish = json!({ "header": header("finish-task"), "payload": { "input": {} } });
    let mut usage = (0, 0);
    let read = |event: &Value| match event["header"]["event"].as_str() {
        Some("result-generated") => {
            if let Some(tokens) = spent(&mut usage, &event["payload"]["usage"]) {
                cfg.meter.tokens(Use::Realtime, &cfg.model, tokens);
            }
            Ok(sentence(&event["payload"]["output"]["sentence"]))
        }
        Some("task-failed") => Err(failed(access, &event["header"])),
        Some("task-finished") => bail!("realtime task finished"),
        _ => Ok(None),
    };
    timed::stream(socket, finish.to_string(), read, speaking, input, deltas).await
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
fn failed(access: &Access, header: &Value) -> anyhow::Error {
    let code = header["error_code"].as_str().unwrap_or_default();
    if code.contains("InvalidApiKey") || code.contains("Unauthorized") {
        let provider = access.provider();
        return TranscriptionError::InvalidKey { provider }.into();
    }
    anyhow::anyhow!("realtime task failed: {header}")
}

async fn connect(
    cfg: &RealtimeConfig,
    access: &Access,
    hints: &Hints,
    run: Value,
) -> anyhow::Result<(Sink, Stream)> {
    let url = root(access)
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let mut request = format!("{url}/api-ws/v1/inference").into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("bearer {}", access.token()))?,
    );
    let (ws, _) = match tokio_tungstenite::connect_async(request).await {
        Ok(connected) => connected,
        Err(e) => {
            if let tungstenite::Error::Http(res) = &e
                && let Some(refused) = relay::refusal(access, res.status())
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
            Some("task-failed") => return Err(failed(access, &event["header"])),
            _ => {}
        }
    }
    bail!("realtime closed before task-started")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sentence_keeps_the_time_of_each_word_and_its_punctuation() {
        let words = json!([
            { "begin_time": 400, "text": "さあ", "punctuation": "" },
            { "begin_time": 900, "text": "始まりました", "punctuation": "。" },
        ]);
        let heard = json!({ "sentence_id": 1, "begin_time": 400, "text": "さあ始まりました。", "words": words });
        let heard = sentence(&heard).unwrap();
        assert_eq!((heard.id, heard.begin), (1, Some(400)));
        let words: Vec<_> = heard
            .words
            .iter()
            .map(|w| (w.at, w.text.as_str()))
            .collect();
        assert_eq!(words, [(Some(400), "さあ"), (Some(900), "始まりました。")]);
        // 文のない知らせ（使用量だけ）
        assert!(sentence(&Value::Null).is_none());
    }

    #[test]
    fn usage_arrives_as_a_running_total() {
        let mut total = (0, 0);
        let usage =
            |input: u64, output: u64| json!({ "input_tokens": input, "output_tokens": output });
        let tokens = spent(&mut total, &usage(57, 10)).unwrap();
        assert_eq!((tokens.input, tokens.output), (57, 10));
        let tokens = spent(&mut total, &usage(120, 25)).unwrap();
        assert_eq!((tokens.input, tokens.output), (63, 15));
        assert!(spent(&mut total, &Value::Null).is_none());
    }
}
