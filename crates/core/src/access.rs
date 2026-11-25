//! API の送り先と鍵。会社へ直接送る（自分のキー。`Access`）か、開発者のサーバーに仕事を頼む（ベータのコード。`Server`。
//! DESIGN.md 17.4）。0.10.0 までは、サーバーが会社ごとに同じパスで中継していて、コードを会社のキーの代わりに使えた

use std::collections::BTreeMap;

use crate::secrets::Provider;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Access {
    provider: Provider,
    /// `https://…/v1` まで（このあとに `/responses` などを付ける）
    base: String,
    /// `Authorization: Bearer` に付ける API キー
    token: String,
}

impl Access {
    pub fn key(provider: Provider, key: String) -> Self {
        let base = provider.info().base;
        Self {
            provider,
            base: base.to_owned(),
            token: key,
        }
    }

    /// 会社と同じ形の口が別の所にあるとき（テストの偽の会社、測定で使う PC の中のモデル）。
    /// `root` の下の、会社の名前（`/openai`、`/xai`）の所に送る
    pub fn at(provider: Provider, key: &str, root: &str) -> Self {
        Self {
            provider,
            base: format!("{}/{}", root.trim_end_matches('/'), provider.info().id),
            token: key.to_owned(),
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
    /// `api` は開発者のサーバーの根元（設定の `apiBase`。コードをもらう口のある `/v1` を指す）。仕事の口は、同じ所の `/v2`
    pub fn new(api: &str, code: String) -> Self {
        Self {
            base: Self::base(api),
            code,
        }
    }

    /// 仕事の口の根元（`https://…/v2`）。コードの要らない口（今のモデル）は、ここから頼む
    pub fn base(api: &str) -> String {
        let root = api.trim_end_matches('/');
        let root = root.strip_suffix("/v1").unwrap_or(root);
        format!("{root}/v2")
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
    fn a_key_goes_straight_to_the_company() {
        let key = Access::key(Provider::OpenAi, "sk".into());
        assert_eq!(key.url("/responses"), "https://api.openai.com/v1/responses");
        assert_eq!(
            key.realtime_url(),
            "wss://api.openai.com/v1/realtime?intent=transcription"
        );
        let xai = Access::key(Provider::Xai, "xai".into());
        assert_eq!(xai.url("/stt"), "https://api.x.ai/v1/stt");
        assert_eq!(xai.provider(), Provider::Xai);
    }

    #[test]
    fn the_task_doors_are_next_to_where_codes_are_given() {
        for api in ["https://api.example/v1", "https://api.example/v1/"] {
            let server = Server::new(api, "K7M2".into());
            assert_eq!(server.url("/translate"), "https://api.example/v2/translate");
        }
        // テストの偽のサーバーは、根元をそのまま渡す
        let server = Server::new("http://127.0.0.1:8787", "K7M2".into());
        assert_eq!(server.url("/status"), "http://127.0.0.1:8787/v2/status");
    }
}
