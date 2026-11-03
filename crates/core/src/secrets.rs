//! API キー。OS の資格情報ストア（Windows の資格情報マネージャー、macOS のキーチェーン）に保存する。
//! 開発中は環境変数（`OPENAI_API_KEY`）が優先する。
//!
//! キーそのものは画面に返さない（どこから読んだかだけを返す）。呼び出しは同期処理なので、
//! 非同期の文脈からは `spawn_blocking` で呼ぶ。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::IDENTIFIER;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Provider {
    /// 文字起こし・翻訳・辞書の漢字名の読み
    #[serde(rename = "openai")]
    OpenAi,
}

impl Provider {
    fn env(self) -> &'static str {
        match self {
            Self::OpenAi => "OPENAI_API_KEY",
        }
    }

    fn account(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum KeySource {
    /// 環境変数（操作パネルからは変えられない）
    Env,
    Keyring,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct KeyStatus {
    #[ts(optional)]
    pub openai: Option<KeySource>,
}

#[derive(Clone, Debug, Default)]
pub struct Keys {
    pub openai: Option<String>,
}

fn get(provider: Provider) -> Option<(String, KeySource)> {
    if let Some(key) = std::env::var(provider.env()).ok().filter(|k| !k.is_empty()) {
        return Some((key, KeySource::Env));
    }
    let entry = keyring::Entry::new(IDENTIFIER, provider.account()).ok()?;
    match entry.get_password() {
        Ok(key) => Some((key, KeySource::Keyring)),
        Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            tracing::warn!("keyring ({}): {e}", provider.account());
            None
        }
    }
}

pub fn load() -> Keys {
    Keys {
        openai: get(Provider::OpenAi).map(|(k, _)| k),
    }
}

pub fn status() -> KeyStatus {
    KeyStatus {
        openai: get(Provider::OpenAi).map(|(_, s)| s),
    }
}

/// 空文字列なら消す
pub fn set(provider: Provider, key: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(IDENTIFIER, provider.account())?;
    let key = key.trim();
    if key.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    } else {
        Ok(entry.set_password(key)?)
    }
}
