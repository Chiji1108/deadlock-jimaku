//! API キーと、ベータのコード。OS の資格情報ストア（Windows の資格情報マネージャー、macOS のキーチェーン、Linux の Secret Service）に保存する。
//! 環境変数は読まない（ほかのツールのために設定した `OPENAI_API_KEY` を黙って使い、思わぬキーに請求しない）。
//!
//! キーそのものは画面に返さない（保存してあるかだけを返す）。呼び出しは同期処理なので、
//! 非同期の文脈からは `spawn_blocking` で呼ぶ。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

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

/// 開発者のサーバーが中継するコードの種類。0.7 までは、人からもらうギフトコード（おすそ分け）もあった
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Relay {
    /// ベータのコード（開発者のキー。ベータ期間中は無料。DESIGN.md 17.2）
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
            Self::OpenAi => ("openai", "OpenAI", "https://api.openai.com/v1", None),
            Self::Xai => ("xai", "xAI", "https://api.x.ai/v1", None),
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
    /// ベータのコード（DESIGN.md 17.2）
    Beta,
}

impl Slot {
    fn account(self) -> &'static str {
        match self {
            Self::Key(provider) => provider.info().id,
            Self::Beta => "beta",
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
    /// ベータのコード。コードそのものは返さない
    pub beta: KeyState,
}

#[derive(Clone, Debug, Default)]
pub struct Keys {
    /// 自分のキー
    pub own: BTreeMap<Provider, String>,
    pub beta: Option<String>,
}

impl Keys {
    /// その会社の送り先と鍵（`api` は開発者のサーバーの根元）。
    /// - ベータのコードが中継する会社（Qwen、OpenRouter）は、コードがあればコードを先に使う。ベータ期間中は無料なので、
    ///   自分のキーを入れている人にも請求しない。コードがなければ自分のキー
    /// - 中継のない会社（OpenAI、xAI、TypeSafe AI）は、自分のキーが要る（なければ、鍵がないとして始めない）
    pub fn access(&self, provider: Provider, api: &str) -> Option<Access> {
        let own = || Some(Access::key(provider, self.own.get(&provider)?.clone()));
        let code = || Some(Access::code(provider, self.beta.clone()?, api));
        match provider.info().relay {
            Some(Relay::Beta) => code().or_else(own),
            None => own(),
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
    let entry = keyring::Entry::new(&crate::keyring_service(), account)?;
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
    let entry = keyring::Entry::new(&crate::keyring_service(), account)?;
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
        beta: get_or_warn(Slot::Beta),
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
        beta: state(Slot::Beta),
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

pub fn beta() -> Option<String> {
    get_or_warn(Slot::Beta)
}

/// 空文字列なら消す
pub fn set_beta(code: &str) -> anyhow::Result<()> {
    put(Slot::Beta, code)
}

/// 0.7 までの「おすそ分け」（人にキーを貸す仕組み。0.8 でなくした）が資格情報ストアに置いたものを消す。
/// もらったギフトコードと、おすそ分けを管理する合言葉。自分が作ったコード（`share-code:{id}`）は、
/// 名前が分からないので残る（サーバーの側で、もう使えなくしてある）
pub fn forget_share() {
    for account in ["gift", "share-token"] {
        if let Err(e) = put_account(account, "") {
            tracing::warn!("keyring ({account}): {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_beta_code_stands_in_for_a_key_only_where_the_server_relays() {
        let keys = Keys {
            own: BTreeMap::from([(Provider::Xai, "xai".to_owned())]),
            beta: None,
        };
        let api = "http://worker/v1";
        // 中継のない会社は、自分のキーだけ
        assert_eq!(keys.access(Provider::OpenAi, api), None);
        assert!(!keys.access(Provider::Xai, api).unwrap().is_code());
        assert_eq!(keys.access(Provider::Qwen, api), None);
        let keys = Keys {
            beta: Some("B3TA".into()),
            ..keys
        };
        assert_eq!(keys.access(Provider::Qwen, api).unwrap().token(), "B3TA");
        assert_eq!(keys.access(Provider::OpenAi, api), None);
        assert_eq!(keys.access(Provider::TypeSafe, api), None);
        // 設定や画面とやりとりする名前は、表の名前と同じ
        for provider in Provider::ALL {
            assert_eq!(serde_json::json!(provider), provider.info().id);
        }
        // ベータのコードがあれば、自分のキーより先に使う（無料の分を先に）。なければ自分のキー
        let own = Keys {
            own: BTreeMap::from([(Provider::Qwen, "sk".to_owned())]),
            ..keys
        };
        assert_eq!(own.access(Provider::Qwen, api).unwrap().token(), "B3TA");
        let without_code = Keys { beta: None, ..own };
        assert!(!without_code.access(Provider::Qwen, api).unwrap().is_code());
    }
}
