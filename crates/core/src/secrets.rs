//! API キーとギフトコードとおすそ分けの合言葉。OS の資格情報ストア（Windows の資格情報マネージャー、macOS のキーチェーン、Linux の Secret Service）に保存する。
//! 環境変数は読まない（ほかのツールのために設定した `OPENAI_API_KEY` を黙って使い、思わぬキーに請求しない）。
//!
//! キーそのものは画面に返さない（保存してあるかだけを返す）。呼び出しは同期処理なので、
//! 非同期の文脈からは `spawn_blocking` で呼ぶ。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::IDENTIFIER;
use crate::openai::Access;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Provider {
    /// 文字起こし・翻訳・辞書の漢字名の読み
    #[serde(rename = "openai")]
    OpenAi,
}

impl Provider {
    fn slot(self) -> Slot {
        match self {
            Self::OpenAi => Slot::OpenAi,
        }
    }
}

/// 資格情報ストアに置くもの（画面から直接は書き換えないものも含む）
#[derive(Clone, Copy, Debug)]
enum Slot {
    OpenAi,
    /// 人からもらったギフトコード（DESIGN.md 17.1）
    Gift,
    /// おすそ分けを管理する合言葉（サーバーが発行）
    ShareToken,
    /// 自分が作ったおすそ分けのコード（コードの ID → コード）。サーバーはハッシュしか持たないので、こちらで覚える
    ShareCodes,
}

impl Slot {
    fn account(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Gift => "gift",
            Self::ShareToken => "share-token",
            Self::ShareCodes => "share-codes",
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
    /// ギフトコード。コードそのものは返さない
    pub gift: KeyState,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keys {
    pub openai: Option<String>,
    pub gift: Option<String>,
}

impl Keys {
    /// 自分のキーがあれば OpenAI へ直接、なければギフトコードで開発者のサーバーを通す（`api` はその根元）
    pub fn access(&self, api: &str) -> Option<Access> {
        match (&self.openai, &self.gift) {
            (Some(key), _) => Some(Access::key(key.clone())),
            (None, Some(code)) => Some(Access::gift(code.clone(), api)),
            (None, None) => None,
        }
    }
}

/// 空の値は保存していないものとして扱う
fn get(slot: Slot) -> Result<Option<String>, keyring::Error> {
    let entry = keyring::Entry::new(IDENTIFIER, slot.account())?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value).filter(|v| !v.trim().is_empty())),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 空文字列なら消す
fn put(slot: Slot, value: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(IDENTIFIER, slot.account())?;
    let value = value.trim();
    if value.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    } else {
        Ok(entry.set_password(value)?)
    }
}

fn get_or_warn(slot: Slot) -> Option<String> {
    get(slot).unwrap_or_else(|e| {
        tracing::warn!("keyring ({}): {e}", slot.account());
        None
    })
}

fn state(slot: Slot) -> KeyState {
    match get(slot) {
        Ok(Some(_)) => KeyState::Saved,
        Ok(None) => KeyState::Missing,
        Err(e) => KeyState::Unreadable {
            reason: e.to_string(),
        },
    }
}

pub fn load() -> Keys {
    Keys {
        openai: get_or_warn(Slot::OpenAi),
        gift: get_or_warn(Slot::Gift),
    }
}

pub fn status() -> KeyStatus {
    KeyStatus {
        openai: state(Slot::OpenAi),
        gift: state(Slot::Gift),
    }
}

/// 空文字列なら消す
pub fn set(provider: Provider, key: &str) -> anyhow::Result<()> {
    put(provider.slot(), key)
}

/// 空文字列なら消す
pub fn set_gift(code: &str) -> anyhow::Result<()> {
    put(Slot::Gift, code)
}

pub fn share_token() -> Option<String> {
    get_or_warn(Slot::ShareToken)
}

/// 空文字列なら消す
pub fn set_share_token(token: &str) -> anyhow::Result<()> {
    put(Slot::ShareToken, token)
}

/// 自分が作ったおすそ分けのコード（ID → コード）
pub fn share_codes() -> BTreeMap<String, String> {
    get_or_warn(Slot::ShareCodes)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn set_share_codes(codes: &BTreeMap<String, String>) -> anyhow::Result<()> {
    if codes.is_empty() {
        return put(Slot::ShareCodes, "");
    }
    put(Slot::ShareCodes, &serde_json::to_string(codes)?)
}
