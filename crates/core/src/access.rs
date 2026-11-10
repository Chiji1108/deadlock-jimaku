//! API の送り先と鍵（DESIGN.md 17.1）。自分のキーがあればその会社へ直接、なければギフトコードで
//! 開発者のサーバー（おすそ分け）を通す。サーバーは会社ごとに同じパスで中継するので、使う側は送り先を替えるだけ

use crate::secrets::Provider;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    provider: Provider,
    /// `https://…/v1` まで（このあとに `/responses` などを付ける）
    base: String,
    /// `Authorization: Bearer` に付けるもの（API キーかギフトコード）
    token: String,
    gift: bool,
}

impl Access {
    pub fn key(provider: Provider, key: String) -> Self {
        let base = match provider {
            Provider::OpenAi => "https://api.openai.com/v1",
            Provider::Xai => "https://api.x.ai/v1",
            // QwenCloud のシンガポールの口（OpenAI 互換）。鍵は地域ごとに別で、ほかの地域の鍵は 401 になる
            Provider::Qwen => "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1",
        };
        Self {
            provider,
            base: base.to_owned(),
            token: key,
            gift: false,
        }
    }

    /// `api` は開発者のサーバーの根元（設定の `apiBase`）。会社ごとに `/openai`、`/xai` の下で中継する
    /// （Qwen の中継はまだない。`Keys::access` は Qwen にギフトコードを使わない）
    pub fn gift(provider: Provider, code: String, api: &str) -> Self {
        let path = match provider {
            Provider::OpenAi => "openai",
            Provider::Xai => "xai",
            Provider::Qwen => "qwen",
        };
        Self {
            provider,
            base: format!("{}/{path}", api.trim_end_matches('/')),
            token: code,
            gift: true,
        }
    }

    pub fn provider(&self) -> Provider {
        self.provider
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Realtime の文字起こしの WebSocket
    pub fn realtime_url(&self) -> String {
        let base = self
            .base
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        format!("{base}/realtime?intent=transcription")
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn is_gift(&self) -> bool {
        self.gift
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gift_goes_through_the_server_on_the_same_paths() {
        let key = Access::key(Provider::OpenAi, "sk".into());
        assert_eq!(key.url("/responses"), "https://api.openai.com/v1/responses");
        assert_eq!(
            key.realtime_url(),
            "wss://api.openai.com/v1/realtime?intent=transcription"
        );
        assert!(!key.is_gift());
        let gift = Access::gift(Provider::OpenAi, "K7M2".into(), "http://127.0.0.1:8787/v1/");
        assert_eq!(
            gift.url("/audio/transcriptions"),
            "http://127.0.0.1:8787/v1/openai/audio/transcriptions"
        );
        assert_eq!(
            gift.realtime_url(),
            "ws://127.0.0.1:8787/v1/openai/realtime?intent=transcription"
        );
        assert!(gift.is_gift());

        let xai = Access::key(Provider::Xai, "xai".into());
        assert_eq!(xai.url("/stt"), "https://api.x.ai/v1/stt");
        let gift = Access::gift(Provider::Xai, "K7M2".into(), "http://127.0.0.1:8787/v1");
        assert_eq!(gift.url("/stt"), "http://127.0.0.1:8787/v1/xai/stt");
        assert_eq!(gift.provider(), Provider::Xai);
    }
}
