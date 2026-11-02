//! 翻訳と名前の読みに使う LLM（OpenAI の chat completions、構造化出力）。
//!
//! 推論はさせない（`reasoning_effort: none`）。比較した結果は DESIGN.md 9.3。

use std::time::Duration;

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const CHAT_URL: &str = "https://api.openai.com/v1/chat/completions";

pub struct Request<'a> {
    pub api_key: &'a str,
    pub model: &'a str,
    pub system: &'a str,
    pub user: String,
    pub schema_name: &'a str,
    pub schema: Value,
    pub timeout: Duration,
}

pub async fn chat_json<T: DeserializeOwned>(
    http: &reqwest::Client,
    req: Request<'_>,
) -> anyhow::Result<T> {
    let body = json!({
        "model": req.model,
        "reasoning_effort": "none",
        "messages": [
            { "role": "system", "content": req.system },
            { "role": "user", "content": req.user },
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": { "name": req.schema_name, "strict": true, "schema": req.schema },
        },
    });
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
    let message = &res["choices"][0]["message"];
    if let Some(refusal) = message["refusal"].as_str() {
        anyhow::bail!("refused: {refusal}");
    }
    let content = message["content"].as_str().context("no message content")?;
    serde_json::from_str(content).context("response does not match the schema")
}
