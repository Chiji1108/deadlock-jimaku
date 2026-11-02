//! Tauri なしで中核だけを起動する（開発・動作確認用）。
//!
//! usage: overlays-headless [--demo]   # --demo: ダミー字幕を流し続ける

use std::net::{Ipv4Addr, SocketAddr};

use overlays_core::{DEFAULT_PORT, demo, server, store::Store};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();

    let store = Store::new();
    if std::env::args().any(|a| a == "--demo") {
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
