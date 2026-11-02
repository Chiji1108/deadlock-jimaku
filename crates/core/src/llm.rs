//! Mistral の chat completions を構造化出力（JSON Schema）で呼ぶ。

use std::time::Duration;

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const CHAT_URL: &str = "https://api.mistral.ai/v1/chat/completions";

pub struct Request<'a> {
    pub api_key: &'a str,
    pub model: &'a str,
    pub system: &'a str,
    pub user: String,
    pub schema_name: &'a str,
    pub schema: Value,
    /// 推論するモデルのときだけ（例: "low"）
    pub reasoning_effort: Option<&'a str>,
    pub timeout: Duration,
}

pub async fn chat_json<T: DeserializeOwned>(
    http: &reqwest::Client,
    req: Request<'_>,
) -> anyhow::Result<T> {
    let mut body = json!({
        "model": req.model,
        "messages": [
            { "role": "system", "content": req.system },
            { "role": "user", "content": req.user },
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": { "name": req.schema_name, "strict": true, "schema": req.schema },
        },
    });
    if let Some(effort) = req.reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }
    let res = http
        .post(CHAT_URL)
        .bearer_auth(req.api_key)
        .timeout(req.timeout)
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    if !status.is_success() {
        anyhow::bail!("{status}: {}", res.text().await.unwrap_or_default());
    }
    let res: Value = res.json().await?;
    let content =
        message_text(&res["choices"][0]["message"]["content"]).context("no message content")?;
    serde_json::from_str(&content).context("response does not match the schema")
}

/// 推論するモデルは content が [{type: "thinking"}, {type: "text"}] の配列になる
fn message_text(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter(|p| p["type"] == "text")
                .filter_map(|p| p["text"].as_str())
                .collect(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_text_handles_thinking_parts() {
        let content = json!([{ "type": "thinking", "thinking": [] }, { "type": "text", "text": "{\"a\":1}" }]);
        assert_eq!(message_text(&content).as_deref(), Some("{\"a\":1}"));
    }
}
