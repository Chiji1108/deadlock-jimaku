//! Tauri なしで中核だけを起動する（開発・動作確認用）。
//!
//! usage: overlays-headless [options]
//!   --demo               ダミー字幕を流し続ける
//!   --input-file PATH    マイクの代わりに WAV ファイルを実時間で流して字幕にする（検証用）
//!
//! 設定は操作パネル（http://127.0.0.1:4510/）で変える（Tauri アプリと同じ settings.json）。
//! API キーは OS の資格情報ストア（操作パネルで設定）か、環境変数 MISTRAL_API_KEY / OPENAI_API_KEY

use std::net::{Ipv4Addr, SocketAddr};

use overlays_core::{
    DEFAULT_PORT, default_data_dir, demo, runtime, server, settings, store::Store,
};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| {
        let i = args.iter().position(|a| a == name)?;
        args.get(i + 1).cloned()
    };

    let data_dir = default_data_dir();
    let settings = settings::load(&data_dir.join(settings::FILE));
    let store = Store::new(settings, data_dir);
    tokio::spawn(runtime::run(
        store.clone(),
        runtime::Options {
            input_file: value("--input-file").map(Into::into),
        },
    ));
    if flag("--demo") {
        let store = store.clone();
        tokio::spawn(async move {
            loop {
                demo::run(&store).await;
            }
        });
    }

    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT))).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    Ok(server::serve(listener, store).await?)
}
