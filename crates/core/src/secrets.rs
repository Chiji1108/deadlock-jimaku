//! API キー。OS の資格情報ストア（Windows の資格情報マネージャー、macOS のキーチェーン、Linux の Secret Service）に保存する。
//! 環境変数は読まない（ほかのツールのために設定した `OPENAI_API_KEY` を黙って使い、思わぬキーに請求しない）。
//!
//! キーそのものは画面に返さない（保存してあるかだけを返す）。呼び出しは同期処理なので、
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
    fn account(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
        }
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(tag = "state", rename_all = "lowercase")]
#[ts(export)]
pub enum KeyState {
    Saved,
    Missing,
    /// 資格情報ストアを読めない（アクセスを拒否した、ロックされている、Secret Service がないなど）
    Unreadable {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct KeyStatus {
    pub openai: KeyState,
}

#[derive(Clone, Debug, Default)]
pub struct Keys {
    pub openai: Option<String>,
}

/// 空のキーは保存していないものとして扱う
fn get(provider: Provider) -> Result<Option<String>, keyring::Error> {
    let entry = keyring::Entry::new(IDENTIFIER, provider.account())?;
    match entry.get_password() {
        Ok(key) => Ok(Some(key).filter(|k| !k.trim().is_empty())),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn load() -> Keys {
    Keys {
        openai: get(Provider::OpenAi).unwrap_or_else(|e| {
            tracing::warn!("keyring ({}): {e}", Provider::OpenAi.account());
            None
        }),
    }
}

pub fn status() -> KeyStatus {
    KeyStatus {
        openai: match get(Provider::OpenAi) {
            Ok(Some(_)) => KeyState::Saved,
            Ok(None) => KeyState::Missing,
            Err(e) => KeyState::Unreadable {
                reason: e.to_string(),
            },
        },
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
