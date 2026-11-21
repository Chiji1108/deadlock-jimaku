//! API の送り先と鍵（DESIGN.md 17.2）。会社へ直接送る（自分のキー）か、開発者のサーバーを通す（ベータのコード）。
//! サーバーは会社ごとに同じパスで中継するので、使う側は送り先を替えるだけ

use std::collections::BTreeMap;

use crate::secrets::Provider;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    provider: Provider,
    /// `https://…/v1` まで（このあとに `/responses` などを付ける）
    base: String,
    /// `Authorization: Bearer` に付けるもの（API キーか、中継するコード）
    token: String,
    code: bool,
}

impl Access {
    pub fn key(provider: Provider, key: String) -> Self {
        let base = provider.info().base;
        Self {
            provider,
            base: base.to_owned(),
            token: key,
            code: false,
        }
    }

    /// `api` は開発者のサーバーの根元（設定の `apiBase`）。会社ごとに `/openai`、`/xai` の下で中継する
    /// （中継のない会社には `Keys::access` がコードを使わない）
    pub fn code(provider: Provider, code: String, api: &str) -> Self {
        Self {
            provider,
            base: format!("{}/{}", api.trim_end_matches('/'), provider.info().id),
            token: code,
            code: true,
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

    pub fn is_code(&self) -> bool {
        self.code
    }
}

/// 開発者のサーバーの、仕事ごとの口（`/v2`。DESIGN.md 17.4）。「何をしてほしいか」を送り、モデルはサーバーが決める。
/// 使う人は、ベータのコード（この PC の印）で見分ける
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    /// `https://…/v2` まで
    base: String,
    code: String,
}

impl Server {
    /// `api` は開発者のサーバーの根元（設定の `apiBase`。会社の形の中継 `/v1` を指す）。仕事の口は、同じ所の `/v2`
    pub fn new(api: &str, code: String) -> Self {
        let root = api.trim_end_matches('/');
        let root = root.strip_suffix("/v1").unwrap_or(root);
        Self {
            base: format!("{root}/v2"),
            code,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub fn code(&self) -> &str {
        &self.code
    }
}

/// 工程が使う送り先と鍵。会社ごとに 1 つと、サーバーの仕事の口（使う工程があるときだけ）
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Accesses {
    providers: BTreeMap<Provider, Access>,
    server: Option<Server>,
}

impl Accesses {
    pub fn new(accesses: impl IntoIterator<Item = Access>) -> Self {
        Self {
            providers: accesses.into_iter().map(|a| (a.provider, a)).collect(),
            server: None,
        }
    }

    pub fn with_server(self, server: Option<Server>) -> Self {
        Self { server, ..self }
    }

    pub fn get(&self, provider: Provider) -> Option<Access> {
        self.providers.get(&provider).cloned()
    }

    pub fn server(&self) -> Option<Server> {
        self.server.clone()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Access> {
        self.providers.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_goes_through_the_server_on_the_same_paths() {
        let key = Access::key(Provider::OpenAi, "sk".into());
        assert_eq!(key.url("/responses"), "https://api.openai.com/v1/responses");
        assert_eq!(
            key.realtime_url(),
            "wss://api.openai.com/v1/realtime?intent=transcription"
        );
        assert!(!key.is_code());
        let code = Access::code(Provider::OpenAi, "K7M2".into(), "http://127.0.0.1:8787/v1/");
        assert_eq!(
            code.url("/audio/transcriptions"),
            "http://127.0.0.1:8787/v1/openai/audio/transcriptions"
        );
        assert_eq!(
            code.realtime_url(),
            "ws://127.0.0.1:8787/v1/openai/realtime?intent=transcription"
        );
        assert!(code.is_code());

        let xai = Access::key(Provider::Xai, "xai".into());
        assert_eq!(xai.url("/stt"), "https://api.x.ai/v1/stt");
        let code = Access::code(Provider::Xai, "K7M2".into(), "http://127.0.0.1:8787/v1");
        assert_eq!(code.url("/stt"), "http://127.0.0.1:8787/v1/xai/stt");
        assert_eq!(code.provider(), Provider::Xai);
    }

    #[test]
    fn the_task_doors_are_next_to_the_relay() {
        for api in ["https://api.example/v1", "https://api.example/v1/"] {
            let server = Server::new(api, "K7M2".into());
            assert_eq!(server.url("/translate"), "https://api.example/v2/translate");
        }
        // テストの偽のサーバーは、根元をそのまま渡す
        let server = Server::new("http://127.0.0.1:8787", "K7M2".into());
        assert_eq!(server.url("/status"), "http://127.0.0.1:8787/v2/status");
    }
}
