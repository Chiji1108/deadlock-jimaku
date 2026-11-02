//! Tauri なしで中核だけを起動する（開発・動作確認用）。
//!
//! usage: overlays-headless [options]
//!   --demo               ダミー字幕を流し続ける
//!   --translate-to en,ru 翻訳先の言語（既定は Settings::default()）
//!   --mic                マイクを字幕にする（MISTRAL_API_KEY が必要）
//!   --mic-device NAME    使うマイク（--mic を兼ねる）
//!   --input-file PATH    WAV ファイルを実時間で流して字幕にする（MISTRAL_API_KEY が必要）

use std::net::{Ipv4Addr, SocketAddr};

use overlays_core::{
    DEFAULT_PORT, assets, audio, default_data_dir, demo, server, settings::Settings, store::Store,
    subtitles,
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

    let mut settings = Settings::default();
    if let Some(langs) = value("--translate-to") {
        settings.translate_to = langs.split(',').map(str::to_owned).collect();
    }
    settings.mic_device = value("--mic-device");
    let input = match value("--input-file") {
        Some(path) => Some(audio::Input::File(path.into())),
        None if flag("--mic") || settings.mic_device.is_some() => {
            Some(audio::Input::Mic(settings.mic_device.clone()))
        }
        None => None,
    };

    let languages = settings.glossary_languages();
    let store = Store::new(settings);
    tokio::spawn(assets::load_into(
        store.clone(),
        default_data_dir().join("assets"),
        languages,
    ));
    if flag("--demo") {
        let store = store.clone();
        tokio::spawn(async move {
            loop {
                demo::run(&store).await;
            }
        });
    }
    if let Some(input) = input {
        let api_key = std::env::var("MISTRAL_API_KEY")
            .map_err(|_| anyhow::anyhow!("MISTRAL_API_KEY is not set"))?;
        let store = store.clone();
        tokio::spawn(async move {
            if let Err(e) = subtitles::run(store, input, api_key).await {
                tracing::error!("transcription stopped: {e:#}");
            }
        });
    }

    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT))).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    Ok(server::serve(listener, store).await?)
}
