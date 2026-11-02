//! Tauri なしで中核だけを起動する（開発・動作確認用）。
//!
//! usage: overlays-headless [--demo]   # --demo: ダミー字幕を流し続ける（翻訳先は en, ru）

use std::net::{Ipv4Addr, SocketAddr};

use overlays_core::{
    DEFAULT_PORT, assets, default_data_dir, demo, server, settings::Settings, store::Store,
};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();

    let demo = std::env::args().any(|a| a == "--demo");
    let settings = Settings {
        // ダミー字幕は英語とロシア語の訳文を出すので、辞書にもその言語を入れる
        translate_to: if demo {
            vec!["en".into(), "ru".into()]
        } else {
            Settings::default().translate_to
        },
        ..Settings::default()
    };

    let store = Store::new();
    tokio::spawn(assets::load_into(
        store.clone(),
        default_data_dir().join("assets"),
        settings.glossary_languages(),
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
