//! OpenAI への送り先と鍵（DESIGN.md 17.1）。自分のキーがあれば OpenAI へ直接、なければギフトコードで
//! 開発者のサーバー（おすそ分け）を通す。サーバーは OpenAI と同じパスで中継するので、使う側は送り先を替えるだけ

/// OpenAI の API の根元
const OPENAI: &str = "https://api.openai.com/v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    /// `https://…/v1` まで（このあとに `/responses` などを付ける）
    base: String,
    /// `Authorization: Bearer` に付けるもの（API キーかギフトコード）
    token: String,
}

impl Access {
    pub fn key(key: String) -> Self {
        Self {
            base: OPENAI.to_owned(),
            token: key,
        }
    }

    /// `api` は開発者のサーバーの根元（設定の `apiBase`）
    pub fn gift(code: String, api: &str) -> Self {
        Self {
            base: format!("{}/openai", api.trim_end_matches('/')),
            token: code,
        }
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
        self.base != OPENAI
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gift_goes_through_the_server_on_the_same_paths() {
        let key = Access::key("sk".into());
        assert_eq!(key.url("/responses"), "https://api.openai.com/v1/responses");
        assert_eq!(
            key.realtime_url(),
            "wss://api.openai.com/v1/realtime?intent=transcription"
        );
        assert!(!key.is_gift());
        let gift = Access::gift("K7M2".into(), "http://127.0.0.1:8787/v1/");
        assert_eq!(
            gift.url("/audio/transcriptions"),
            "http://127.0.0.1:8787/v1/openai/audio/transcriptions"
        );
        assert_eq!(
            gift.realtime_url(),
            "ws://127.0.0.1:8787/v1/openai/realtime?intent=transcription"
        );
        assert!(gift.is_gift());
    }
}
