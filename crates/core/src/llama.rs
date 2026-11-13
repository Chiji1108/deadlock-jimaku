//! PC の中で動く LLM（Gemma 4 E4B）。llama.cpp（MIT）の `llama-server` を別のプロセスで立て、
//! OpenAI と同じ形の口で頼む（DESIGN.md 9.7）。翻訳（`mt`）と用語の判定（`decide`）が使う。
//!
//! llama.cpp は初めて使うときに GitHub のリリースから、モデル（Apache 2.0、約 5.2 GB）は Hugging Face から取り、
//! どちらも SHA-256 を確かめてからデータフォルダに置く（`download.rs`）。文字はこの PC から出ない。
//! プロセスは字幕を止めると終わる

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use serde_json::{Value, json};
use tokio::process::Command;

use crate::download::{self, Model, Package};
use crate::sidecar::{self, Sidecar};
use crate::usage::Tokens;

/// 料金の表に使う名前（無料）
pub const GEMMA: &str = "gemma-4-e4b";

/// Google の公式の GGUF（量子化を見込んで鍛えた 4 bit）。測ったのはこのファイル（DESIGN.md 9.7）
const MODEL: Model = Model {
    url: "https://huggingface.co/google/gemma-4-E4B-it-qat-q4_0-gguf/resolve/4b4a2c1d584be7264f87aac328a1bc739ce81b6c/gemma-4-E4B_q4_0-it.gguf",
    sha256: "676c35070db6dbe52f93e9c864ee0fba4eddea94b9c875d9cb10daff453fbaee",
    file: "gemma-4-E4B_q4_0-it.gguf",
};

/// llama.cpp 0.9.0 に当たるビルド
const BUILD: &str = "b12110";

/// この PC で動く配布物の名前、SHA-256、広げた中の `llama-server` の場所。Intel の Mac では GPU を使えないので配らない
const RELEASE: Option<(&str, &str, &str)> =
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some((
            "macos-arm64.tar.gz",
            "4f5b8de12e0b9516658ca6a5764c6ffb3d7c659c3dd538556b1413c7b4050662",
            "llama-b12110/llama-server",
        ))
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some((
            "win-vulkan-x64.zip",
            "9732a5e48887ca54d0bfe4af4324dc85fd56cd8cda19b52df3542fa6ebe6874f",
            "llama-server.exe",
        ))
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some((
            "ubuntu-x64.tar.gz",
            "368227edcc229be5f9bed16a2223d79b5e41d0ac2e19bcf5825e82291c060e55",
            "llama-b12110/llama-server",
        ))
    } else {
        None
    };

/// 同時に答える数。翻訳先（最大 4）と、1 つの文の用語の問いを並べて聞く
const SLOTS: u32 = 4;

/// 1 つの問い合わせが読める長さ（トークン）。字幕の 1 文と前後の文、用語の説明には十分
const CONTEXT_PER_SLOT: u32 = 2048;

/// モデルを GPU に載せるまでの待ち時間（ファイルを取る時間は含まない）
const READY_TIMEOUT: Duration = Duration::from_secs(5 * 60);

static RUNNING: Sidecar = Sidecar::new("llama.cpp");

/// llama.cpp を終わらせる
pub fn stop() {
    RUNNING.stop();
}

/// 動いている llama.cpp のサーバー
pub struct Llama {
    port: u16,
}

impl Drop for Llama {
    fn drop(&mut self) {
        stop();
    }
}

/// 1 回の頼みごと
pub struct Prompt<'a> {
    pub system: &'a str,
    pub user: &'a str,
    /// 書ける文字を縛る文法（llama.cpp の GBNF）
    pub grammar: &'a str,
    pub max_tokens: u32,
    pub timeout: Duration,
}

impl Llama {
    /// `dir` に llama.cpp とモデルを置き（なければ取る）、サーバーを立てて、頼めるようになるまで待つ。
    /// `progress` に、モデルを取った量と全体の量（バイト）を知らせる
    pub async fn start(dir: &Path, progress: &(dyn Fn(u64, u64) + Sync)) -> anyhow::Result<Self> {
        let (target, sha256, file) =
            RELEASE.context("llama.cpp has no build for this computer (Gemma cannot run here)")?;
        let url = format!(
            "https://github.com/ggml-org/llama.cpp/releases/download/{BUILD}/llama-{BUILD}-bin-{target}"
        );
        let package = Package {
            url: &url,
            sha256,
            folder: BUILD,
            file,
        };
        let executable = download::installed(dir, &package).await?;
        let model = download::model(&dir.join("models"), &MODEL, progress).await?;
        let port = sidecar::free_port()?;
        let mut command = Command::new(&executable);
        command
            .arg("--model")
            .arg(&model)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            // 全部の層を GPU に載せる（GPU がなければ CPU で動く）
            .args(["--n-gpu-layers", "99"])
            .args(["--ctx-size", &(SLOTS * CONTEXT_PER_SLOT).to_string()])
            .args(["--parallel", &SLOTS.to_string()])
            .arg("--no-webui");
        // ここから先で失敗しても、待っている間に捨てられても、プロセスを残さない
        let llama = Self { port };
        RUNNING.serve(command, port, READY_TIMEOUT).await?;
        tracing::info!("llama.cpp {BUILD} serves {GEMMA} on port {port}");
        Ok(llama)
    }

    /// 文法で縛った答えと、使ったトークン。答える前に考えさせない（字幕には遅すぎる）
    pub async fn complete(&self, prompt: &Prompt<'_>) -> anyhow::Result<(String, Tokens)> {
        let body = json!({
            "messages": [
                { "role": "system", "content": prompt.system },
                { "role": "user", "content": prompt.user },
            ],
            "temperature": 0,
            "max_tokens": prompt.max_tokens,
            "grammar": prompt.grammar,
            "cache_prompt": true,
            "chat_template_kwargs": { "enable_thinking": false },
        });
        let url = format!("http://127.0.0.1:{}/v1/chat/completions", self.port);
        let res = sidecar::client()
            .post(url)
            .timeout(prompt.timeout)
            .json(&body)
            .send()
            .await?;
        let status = res.status();
        if !status.is_success() {
            anyhow::bail!("{status}: {}", res.text().await.unwrap_or_default());
        }
        let res: Value = res.json().await?;
        let n = |key: &str| res["usage"][key].as_u64().unwrap_or(0);
        let tokens = Tokens {
            input: n("prompt_tokens"),
            cached: 0,
            output: n("completion_tokens"),
        };
        let text = res["choices"][0]["message"]["content"].as_str();
        Ok((text.context("no answer")?.trim().to_owned(), tokens))
    }
}
