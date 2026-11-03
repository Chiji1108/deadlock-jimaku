//! 翻訳と名前の読みに使う LLM（OpenAI の chat completions、構造化出力）。
//!
//! 推論の深さは設定（`models.reasoning_effort`、既定 `none`）。比較した結果は DESIGN.md 9.3。

use std::time::Duration;

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::usage::{Meter, Tokens, Use};

const CHAT_URL: &str = "https://api.openai.com/v1/chat/completions";

pub struct Request<'a> {
    pub api_key: &'a str,
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
    req.meter
        .tokens(req.purpose, req.model, Tokens::from_response(&res["usage"]));
    let message = &res["choices"][0]["message"];
    if let Some(refusal) = message["refusal"].as_str() {
        anyhow::bail!("refused: {refusal}");
    }
    let content = message["content"].as_str().context("no message content")?;
    serde_json::from_str(content).context("response does not match the schema")
}
