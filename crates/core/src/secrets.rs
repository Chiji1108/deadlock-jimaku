//! API キーとギフトコードとおすそ分けの合言葉。OS の資格情報ストア（Windows の資格情報マネージャー、macOS のキーチェーン、Linux の Secret Service）に保存する。
//! 環境変数は読まない（ほかのツールのために設定した `OPENAI_API_KEY` を黙って使い、思わぬキーに請求しない）。
//!
//! キーそのものは画面に返さない（保存してあるかだけを返す）。呼び出しは同期処理なので、
//! 非同期の文脈からは `spawn_blocking` で呼ぶ。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::IDENTIFIER;
use std::collections::BTreeMap;

use crate::access::{Access, Accesses};
use crate::settings::Engines;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
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
    /// いろいろな会社のモデルを 1 つの鍵で呼べる（判定モデル、Hy-MT2、MAI-Transcribe 2。DESIGN.md 9.7）
    #[serde(rename = "openrouter")]
    OpenRouter,
    /// 用語の判定（Jev）
    #[serde(rename = "typesafe")]
    TypeSafe,
}

/// 会社ごとの決まり。会社を足すときは、ここと `Provider` に足す（画面の一覧は `settings::catalog` から作る）
pub struct ProviderInfo {
    /// 資格情報ストアでの名前。設定や画面とのやりとりの名前（`Provider` の serde の名前）も、
    /// 開発者のサーバーが中継するパスも同じ
    pub id: &'static str,
    /// 画面に出す名前
    pub name: &'static str,
    /// 自分のキーで送る先。`https://…/v1` まで
    pub base: &'static str,
    /// 開発者のサーバーが中継するなら、どのコードで使えるか。None の会社は自分のキーが要る。
    /// そのコードがその会社を中継するかは、字幕を始めるときにサーバーに確かめる
    pub relay: Option<Relay>,
}

/// 開発者のサーバーが中継するコードの種類。アプリはどちらも同じ所（ギフトコード）に保存する
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Relay {
    /// 人からもらったギフトコード（おすそ分けした人のキー。DESIGN.md 17.1）
    Gift,
    /// ベータのコード（開発者のキー。ベータの間は無料。DESIGN.md 17.2）
    Beta,
}

impl Provider {
    /// 鍵が要る順（足りない鍵は OpenAI を先に知らせる）
    pub const ALL: [Self; 5] = [
        Self::OpenAi,
        Self::Xai,
        Self::Qwen,
        Self::OpenRouter,
        Self::TypeSafe,
    ];

    pub const fn info(self) -> ProviderInfo {
        let (id, name, base, relay) = match self {
            Self::OpenAi => (
                "openai",
                "OpenAI",
                "https://api.openai.com/v1",
                Some(Relay::Gift),
            ),
            Self::Xai => ("xai", "xAI", "https://api.x.ai/v1", Some(Relay::Gift)),
            // QwenCloud のシンガポールの口（OpenAI 互換）。鍵は地域ごとに別で、ほかの地域の鍵は 401 になる
            Self::Qwen => (
                "qwen",
                "Qwen",
                "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1",
                Some(Relay::Beta),
            ),
            Self::OpenRouter => (
                "openrouter",
                "OpenRouter",
                "https://openrouter.ai/api/v1",
                Some(Relay::Beta),
            ),
            Self::TypeSafe => (
                "typesafe",
                "TypeSafe AI",
                "https://api.typesafe.ai/v1",
                None,
            ),
        };
        ProviderInfo {
            id,
            name,
            base,
            relay,
        }
    }
}

/// 資格情報ストアに置くもの（画面から直接は書き換えないものも含む）
#[derive(Clone, Copy, Debug)]
enum Slot {
    Key(Provider),
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
            Self::Key(provider) => provider.info().id,
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
    pub openrouter: KeyState,
    pub typesafe: KeyState,
    /// ギフトコード。コードそのものは返さない
    pub gift: KeyState,
}

#[derive(Clone, Debug, Default)]
pub struct Keys {
    /// 自分のキー
    pub own: BTreeMap<Provider, String>,
    pub gift: Option<String>,
}

impl Keys {
    /// 自分のキーがあればその会社へ直接、なければギフトコードで開発者のサーバーを通す（`api` はその根元）。
    /// ギフトコードでその会社を使えるか（おすそ分けした人がその会社のキーを預けているか）は、字幕を始めるときに確かめる。
    /// 中継のない会社は、ギフトコードでは足りない（鍵がないとして始めない）
    pub fn access(&self, provider: Provider, api: &str) -> Option<Access> {
        match (self.own.get(&provider), &self.gift) {
            (Some(key), _) => Some(Access::key(provider, key.clone())),
            (None, Some(code)) if provider.info().relay.is_some() => {
                Some(Access::gift(provider, code.clone(), api))
            }
            _ => None,
        }
    }

    /// 工程が使う会社の送り先と鍵。使わない会社には何も送らない
    pub fn accesses(&self, engines: &Engines, api: &str) -> Accesses {
        let providers = engines.providers().into_iter();
        Accesses::new(providers.filter_map(|provider| self.access(provider, api)))
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
    let own = Provider::ALL.into_iter();
    Keys {
        own: own
            .filter_map(|provider| Some((provider, get_or_warn(Slot::Key(provider))?)))
            .collect(),
        gift: get_or_warn(Slot::Gift),
    }
}

pub fn status() -> KeyStatus {
    let key = |provider| state(Slot::Key(provider));
    KeyStatus {
        openai: key(Provider::OpenAi),
        xai: key(Provider::Xai),
        qwen: key(Provider::Qwen),
        openrouter: key(Provider::OpenRouter),
        typesafe: key(Provider::TypeSafe),
        gift: state(Slot::Gift),
    }
}

/// 空文字列なら消す
pub fn set(provider: Provider, key: &str) -> anyhow::Result<()> {
    put(Slot::Key(provider), key)
}

/// その会社の自分のキー
pub fn key(provider: Provider) -> Option<String> {
    get_or_warn(Slot::Key(provider))
}

pub fn openai() -> Option<String> {
    get_or_warn(Slot::Key(Provider::OpenAi))
}

pub fn xai() -> Option<String> {
    get_or_warn(Slot::Key(Provider::Xai))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gift_stands_in_for_a_missing_key_only_where_the_server_relays() {
        let keys = Keys {
            own: BTreeMap::from([(Provider::Xai, "xai".to_owned())]),
            gift: Some("K7M2".into()),
        };
        let api = "http://worker/v1";
        assert!(keys.access(Provider::OpenAi, api).unwrap().is_gift());
        assert!(!keys.access(Provider::Xai, api).unwrap().is_gift());
        assert!(keys.access(Provider::Qwen, api).unwrap().is_gift());
        assert_eq!(keys.access(Provider::TypeSafe, api), None);
        // 設定や画面とやりとりする名前は、表の名前と同じ
        for provider in Provider::ALL {
            assert_eq!(serde_json::json!(provider), provider.info().id);
        }
        let own = Keys {
            own: BTreeMap::from([(Provider::Qwen, "sk".to_owned())]),
            ..keys
        };
        assert!(!own.access(Provider::Qwen, api).unwrap().is_gift());
    }
}
