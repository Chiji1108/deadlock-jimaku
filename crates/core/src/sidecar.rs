//! PC の中で動かす別のプロセス（speech.cpp、llama.cpp）。この PC の中の口で頼む。
//!
//! プログラムごとに、同時に動くのは 1 つ（字幕の処理は 1 つしか動かない）。字幕を止めるか、アプリを終わるときに終わらせる

use std::net::{Ipv4Addr, TcpListener};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, bail};
use tokio::process::{Child, Command};

pub struct Sidecar {
    /// 記録に出す名前
    name: &'static str,
    running: Mutex<Option<Child>>,
}

impl Sidecar {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            running: Mutex::new(None),
        }
    }

    /// プロセスを終わらせる
    pub fn stop(&self) {
        if let Some(mut child) = self.running.lock().unwrap().take() {
            let _ = child.start_kill();
        }
    }

    /// `command`（`port` で待つサーバー）を立てて、`GET /health` が通るまで待つ。前のプロセスは終わらせる。
    /// 失敗しても、待っている間に捨てられても、プロセスを残さないよう、呼ぶ側は先に止める手段（Drop）を持っておく
    pub async fn serve(
        &self,
        mut command: Command,
        port: u16,
        wait: Duration,
    ) -> anyhow::Result<()> {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // このタスクごと捨てられても残さない（ふだんは `stop` が終わらせる）
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW（コンソールの窓を出さない）
        let name = self.name;
        let child = command
            .spawn()
            .with_context(|| format!("cannot start {name}"))?;
        self.stop();
        *self.running.lock().unwrap() = Some(child);
        let health = format!("http://127.0.0.1:{port}/health");
        let ready = async {
            loop {
                let exited = match &mut *self.running.lock().unwrap() {
                    Some(child) => child.try_wait()?,
                    None => bail!("{name} was stopped"),
                };
                if let Some(status) = exited {
                    bail!("{name} exited: {status}");
                }
                let res = client().get(&health).send().await;
                if res.is_ok_and(|res| res.status().is_success()) {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        };
        tokio::time::timeout(wait, ready)
            .await
            .with_context(|| format!("{name} did not get ready"))?
    }
}

/// 空いているポート（プロセスに選ばせると、出力から読み取ることになる）
pub fn free_port() -> anyhow::Result<u16> {
    Ok(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
        .local_addr()?
        .port())
}

/// この PC の中の口に送る。プロキシの設定があっても通さない
pub fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        let client = reqwest::Client::builder().no_proxy();
        client.build().expect("valid client config")
    })
}
