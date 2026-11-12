//! Tauri なしで中核だけを起動する（開発・動作確認用）。
//!
//! usage: overlays-headless [options]
//!   --demo               ダミー字幕を流し続ける
//!   --input-file PATH    マイクの代わりに WAV ファイルを実時間で流して字幕にする（検証用）
//!
//! 設定は操作パネル（http://127.0.0.1:4510/）で変える（Tauri アプリと同じ settings.json）。
//! API キーは OS の資格情報ストア（操作パネルで設定）

use std::net::{Ipv4Addr, SocketAddr};

use anyhow::Context;
use overlays_core::{default_data_dir, demo, port, runtime, server, settings, store::Store};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // 知らない引数は黙って無視しない（以前の --rank-account などは操作パネルに移った）
    let mut demo_mode = false;
    let mut input_file = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--demo" => demo_mode = true,
            "--input-file" => input_file = Some(args.next().context("--input-file needs a path")?),
            _ => anyhow::bail!(
                "unknown argument {arg:?}. usage: overlays-headless [--demo] [--input-file PATH]\n\
                 settings (mic, languages, rank account) are in the control panel: http://127.0.0.1:{}/",
                port()
            ),
        }
    }

    let data_dir = default_data_dir();
    let settings = settings::load(&data_dir.join(settings::FILE));
    let store = Store::new(settings, data_dir);
    tokio::spawn(runtime::run(
        store.clone(),
        runtime::Options {
            input_file: input_file.map(Into::into),
        },
    ));
    if demo_mode {
        let store = store.clone();
        tokio::spawn(async move {
            loop {
                demo::run(&store).await;
            }
        });
    }

    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port()))).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    Ok(server::serve(listener, store).await?)
}
