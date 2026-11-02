//! Tauri なしで中核だけを起動する（開発・動作確認用）。
//!
//! usage: overlays-headless [--demo] [--translate-to en,ru]
//!   --demo          ダミー字幕を流し続ける
//!   --translate-to  翻訳先の言語（既定は Settings::default()）

use std::net::{Ipv4Addr, SocketAddr};

use overlays_core::{
    DEFAULT_PORT, assets, default_data_dir, demo, server, settings::Settings, store::Store,
};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();
    let demo = args.iter().any(|a| a == "--demo");
    let mut settings = Settings::default();
    if let Some(langs) = args
        .iter()
        .position(|a| a == "--translate-to")
        .and_then(|i| args.get(i + 1))
    {
        settings.translate_to = langs.split(',').map(str::to_owned).collect();
    }

    let languages = settings.glossary_languages();
    let store = Store::new(settings);
    tokio::spawn(assets::load_into(
        store.clone(),
        default_data_dir().join("assets"),
        languages,
    ));
    if demo {
        let store = store.clone();
        tokio::spawn(async move {
            loop {
                demo::run(&store).await;
            }
        });
    }

    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT))).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    server::serve(listener, store).await
}
