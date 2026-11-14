//! 用語・翻訳・ふりがな・名前の読みに使う LLM（OpenAI の Responses API、構造化出力）。
//!
//! 推論の深さは設定（`models.reasoning_effort`、既定 `none`）。比較した結果は DESIGN.md 9.3。
//! 応答は前の応答を引き継いで使わないので、OpenAI に保存させない（`store: false`）

use std::time::Duration;

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::access::Access;
use crate::relay;
use crate::usage::{Meter, Tokens, Use};

pub struct Request<'a> {
    pub access: &'a Access,
    pub model: &'a str,
    /// 推論しないモデルでは None（パラメータを送らない）
    pub reasoning_effort: Option<&'a str>,
    pub system: &'a str,
    pub user: String,
    pub schema_name: &'a str,
    pub schema: Value,
    pub timeout: Duration,
    /// 使ったトークンを数える
    pub meter: &'a Meter,
    pub purpose: Use,
}

pub async fn chat_json<T: DeserializeOwned>(
    http: &reqwest::Client,
    req: Request<'_>,
) -> anyhow::Result<T> {
    let res = send(http, &req, body(&req)).await?;
    finish(&req, &res.json().await?)
}

/// `chat_json` と同じ答えを返す。途中の文（JSON の書きかけ）を、届くたびに `on_text` に渡す
pub async fn stream_json<T: DeserializeOwned>(
    http: &reqwest::Client,
    req: Request<'_>,
    mut on_text: impl FnMut(&str),
) -> anyhow::Result<T> {
    let mut body = body(&req);
    body["stream"] = json!(true);
    let mut res = send(http, &req, body).await?;
    let mut text = String::new();
    let mut buf = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        buf.extend_from_slice(&chunk);
        while let Some(end) = buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buf.drain(..=end).collect();
            let Some(event) = event(&line) else {
                continue;
            };
            match event["type"].as_str() {
                Some("response.output_text.delta") => {
                    text.push_str(event["delta"].as_str().unwrap_or_default());
                    on_text(&text);
                }
                Some("response.completed" | "response.incomplete" | "response.failed") => {
                    return finish(&req, &event["response"]);
                }
                Some("error") => anyhow::bail!("stream: {event}"),
                _ => {}
            }
        }
    }
    anyhow::bail!("the stream ended without a response")
}

/// SSE の 1 行。`data:` の行だけが JSON を持つ
fn event(line: &[u8]) -> Option<Value> {
    serde_json::from_slice(line.strip_prefix(b"data:")?).ok()
}

fn body(req: &Request<'_>) -> Value {
    let mut body = json!({
        "model": req.model,
        "instructions": req.system,
        "input": req.user,
        "text": {
            "format": { "type": "json_schema", "name": req.schema_name, "strict": true, "schema": req.schema },
        },
        "store": false,
    });
    if let Some(effort) = req.reasoning_effort {
        body["reasoning"] = json!({ "effort": effort });
    }
    body
}

async fn send(
    http: &reqwest::Client,
    req: &Request<'_>,
    body: Value,
) -> anyhow::Result<reqwest::Response> {
    let res = http
        .post(req.access.url("/responses"))
        .bearer_auth(req.access.token())
        .timeout(req.timeout)
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    if !status.is_success() {
        // 鍵やコードが断られたら、確定の文字起こしと同じく字幕を止める理由にする（DESIGN.md 17.2）
        let body = res.text().await.unwrap_or_default();
        let reason = relay::refusal_reason(&body);
        if let Some(refused) = relay::refusal(req.access, status, reason.as_deref()) {
            return Err(refused.into());
        }
        anyhow::bail!("{status}: {body}");
    }
    Ok(res)
}

fn finish<T: DeserializeOwned>(req: &Request<'_>, res: &Value) -> anyhow::Result<T> {
    req.meter
        .tokens(req.purpose, req.model, Tokens::from_response(&res["usage"]));
    serde_json::from_str(&output_text(res)?).context("response does not match the schema")
}

/// 応答の文。`output` のメッセージの `output_text` をつなげる（推論の項目などは飛ばす）
fn output_text(res: &Value) -> anyhow::Result<String> {
    let contents = res["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["type"] == "message")
        .flat_map(|item| item["content"].as_array().into_iter().flatten());
    let mut text = String::new();
    for content in contents {
        match content["type"].as_str() {
            Some("output_text") => text.push_str(content["text"].as_str().unwrap_or_default()),
            Some("refusal") => anyhow::bail!("refused: {}", content["refusal"]),
            _ => {}
        }
    }
    anyhow::ensure!(
        !text.is_empty(),
        "no output text: {} {}",
        res["status"],
        res["error"]
    );
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_reads_only_data_lines() {
        assert_eq!(event(b"data: {\"type\":\"x\"}\n").unwrap()["type"], "x");
        assert!(event(b"event: response.created\n").is_none());
        assert!(event(b"data: [DONE]\n").is_none());
        assert!(event(b"\n").is_none());
    }

    #[test]
    fn output_text_joins_message_texts_and_reports_refusals() {
        let res = json!({ "output": [
            { "type": "reasoning", "summary": [] },
            { "type": "message", "content": [{ "type": "output_text", "text": "{\"a\":" }, { "type": "output_text", "text": "1}" }] },
        ]});
        assert_eq!(output_text(&res).unwrap(), "{\"a\":1}");
        let refused = json!({ "output": [{ "type": "message", "content": [{ "type": "refusal", "refusal": "no" }] }] });
        assert!(
            output_text(&refused)
                .unwrap_err()
                .to_string()
                .contains("refused")
        );
        assert!(output_text(&json!({ "status": "incomplete", "output": [] })).is_err());
    }
}
