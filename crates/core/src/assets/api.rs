//! deadlock-api のアセット API（DESIGN.md 8.1）。使うフィールドだけを読む。

use std::collections::BTreeMap;

use anyhow::Context;
use serde::Deserialize;

const BASE: &str = "https://api.deadlock-api.com/v1/assets";

#[derive(Clone, Debug, Deserialize)]
pub struct ApiHero {
    pub id: u32,
    pub class_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub player_selectable: bool,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub images: BTreeMap<String, serde_json::Value>,
    /// スロット名 → class_name（`signature1`〜`signature4` がスキル）
    #[serde(default)]
    pub items: BTreeMap<String, Option<String>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApiItem {
    pub id: u32,
    pub class_name: String,
    #[serde(default)]
    pub name: Option<String>,
    /// `upgrade` / `ability` / `weapon`
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub shopable: bool,
    #[serde(default)]
    pub item_slot_type: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub image_webp: Option<String>,
    #[serde(default)]
    pub shop_image: Option<String>,
    #[serde(default)]
    pub shop_image_webp: Option<String>,
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            http: crate::http().clone(),
        }
    }
}

impl Client {
    pub async fn latest_client_version(&self) -> anyhow::Result<u32> {
        let versions: Vec<u32> = self.get_json(&format!("{BASE}/client-versions")).await?;
        versions
            .into_iter()
            .max()
            .context("client-versions is empty")
    }

    pub async fn heroes(&self, language: &str, version: u32) -> anyhow::Result<Vec<ApiHero>> {
        self.get_json(&format!(
            "{BASE}/heroes?language={language}&client_version={version}"
        ))
        .await
    }

    pub async fn items(&self, language: &str, version: u32) -> anyhow::Result<Vec<ApiItem>> {
        self.get_json(&format!(
            "{BASE}/items?language={language}&client_version={version}"
        ))
        .await
    }

    pub async fn bytes(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        Ok(self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec())
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> anyhow::Result<T> {
        self.http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .with_context(|| format!("GET {url}"))
    }
}
