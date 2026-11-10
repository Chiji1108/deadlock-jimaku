//! API キーとギフトコードとおすそ分けの合言葉。OS の資格情報ストア（Windows の資格情報マネージャー、macOS のキーチェーン、Linux の Secret Service）に保存する。
//! 環境変数は読まない（ほかのツールのために設定した `OPENAI_API_KEY` を黙って使い、思わぬキーに請求しない）。
//!
//! キーそのものは画面に返さない（保存してあるかだけを返す）。呼び出しは同期処理なので、
//! 非同期の文脈からは `spawn_blocking` で呼ぶ。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::IDENTIFIER;
use crate::access::Access;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Provider {
    /// 文字起こし・翻訳・辞書の漢字名の読み
    #[serde(rename = "openai")]
    OpenAi,
    /// 確定の文字起こし（Grok Voice Transcribe）
    Xai,
    /// 翻訳（Qwen-MT-Flash）。QwenCloud のシンガポールの鍵（DESIGN.md 9.7）
    Qwen,
}

impl Provider {
    fn slot(self) -> Slot {
        match self {
            Self::OpenAi => Slot::OpenAi,
            Self::Xai => Slot::Xai,
            Self::Qwen => Slot::Qwen,
        }
    }
}

/// 資格情報ストアに置くもの（画面から直接は書き換えないものも含む）
#[derive(Clone, Copy, Debug)]
enum Slot {
    OpenAi,
    Xai,
    Qwen,
    /// 人からもらったギフトコード（DESIGN.md 17.1）
    Gift,
    /// おすそ分けを管理する合言葉（サーバーが発行）
    ShareToken,
    /// 自分が作ったおすそ分けのコード（1 つずつ。サーバーはハッシュしか持たないので、こちらで覚える）
    ShareCode,
}

impl Slot {
    fn account(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Xai => "xai",
            Self::Qwen => "qwen",
            Self::Gift => "gift",
            Self::ShareToken => "share-token",
            Self::ShareCode => "share-code",
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
    pub xai: KeyState,
    pub qwen: KeyState,
    /// ギフトコード。コードそのものは返さない
    pub gift: KeyState,
}

#[derive(Clone, Debug, Default)]
pub struct Keys {
    pub openai: Option<String>,
    pub xai: Option<String>,
    pub qwen: Option<String>,
    pub gift: Option<String>,
}

impl Keys {
    /// 自分のキーがあればその会社へ直接、なければギフトコードで開発者のサーバーを通す（`api` はその根元）。
    /// ギフトコードでその会社を使えるか（おすそ分けした人がその会社のキーを預けているか）は、字幕を始めるときに確かめる
    pub fn access(&self, provider: Provider, api: &str) -> Option<Access> {
        let own = match provider {
            Provider::OpenAi => &self.openai,
            Provider::Xai => &self.xai,
            Provider::Qwen => &self.qwen,
        };
        match (own, &self.gift) {
            (Some(key), _) => Some(Access::key(provider, key.clone())),
            (None, Some(code)) => Some(Access::gift(provider, code.clone(), api)),
            (None, None) => None,
        }
    }
}

/// 空の値は保存していないものとして扱う
fn get(slot: Slot) -> Result<Option<String>, keyring::Error> {
    get_account(slot.account())
}

fn get_account(account: &str) -> Result<Option<String>, keyring::Error> {
    let entry = keyring::Entry::new(IDENTIFIER, account)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value).filter(|v| !v.trim().is_empty())),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 空文字列なら消す
fn put(slot: Slot, value: &str) -> anyhow::Result<()> {
    put_account(slot.account(), value)
}

fn put_account(account: &str, value: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(IDENTIFIER, account)?;
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
        xai: get_or_warn(Slot::Xai),
        qwen: get_or_warn(Slot::Qwen),
        gift: get_or_warn(Slot::Gift),
    }
}

pub fn status() -> KeyStatus {
    KeyStatus {
        openai: state(Slot::OpenAi),
        xai: state(Slot::Xai),
        qwen: state(Slot::Qwen),
        gift: state(Slot::Gift),
    }
}

/// 空文字列なら消す
pub fn set(provider: Provider, key: &str) -> anyhow::Result<()> {
    put(provider.slot(), key)
}

/// その会社の自分のキー
pub fn key(provider: Provider) -> Option<String> {
    get_or_warn(provider.slot())
}

pub fn openai() -> Option<String> {
    get_or_warn(Slot::OpenAi)
}

pub fn xai() -> Option<String> {
    get_or_warn(Slot::Xai)
}

pub fn gift() -> Option<String> {
    get_or_warn(Slot::Gift)
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

/// 自分が作ったおすそ分けのコード。この PC で作っていなければ None
pub fn share_code(id: &str) -> Option<String> {
    let account = format!("{}:{id}", Slot::ShareCode.account());
    get_account(&account).unwrap_or_else(|e| {
        tracing::warn!("keyring ({account}): {e}");
        None
    })
}

/// 空文字列なら消す
pub fn set_share_code(id: &str, code: &str) -> anyhow::Result<()> {
    put_account(&format!("{}:{id}", Slot::ShareCode.account()), code)
}
