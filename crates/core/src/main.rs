//! Tauri なしで中核だけを起動する（開発・動作確認用）。
//!
//! usage: overlays-headless [options]
//!   --demo               ダミー字幕を流し続ける
//!   --translate-to en,ru 翻訳先の言語（既定は Settings::default()）
//!   --mic                マイクを字幕にする（Mistral の API キーが必要）
//!   --mic-device NAME    使うマイク（--mic を兼ねる）
//!   --input-file PATH    WAV ファイルを実時間で流して字幕にする（Mistral の API キーが必要）
//!   --rank-account ID    ランクを表示するアカウント（SteamID64 か 32bit のアカウント ID）
//!
//! 設定は操作パネル（http://127.0.0.1:4510/）と同じ settings.json を読む。引数はその上書き。
//! API キーは OS の資格情報ストア（操作パネルで設定）か、環境変数 MISTRAL_API_KEY / OPENAI_API_KEY

use std::net::{Ipv4Addr, SocketAddr};

use overlays_core::{
    DEFAULT_PORT, default_data_dir, demo, runtime, server,
    settings::{self},
    store::Store,
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

    // 保存された設定に、引数で上書きする（上書きはこの起動の間だけ。操作パネルで変えると保存される）
    let data_dir = default_data_dir();
    let mut settings = settings::load(&data_dir.join(settings::FILE));
    if let Some(langs) = value("--translate-to") {
        settings.translate_to = langs.split(',').map(str::to_owned).collect();
    }
    if let Some(device) = value("--mic-device") {
        settings.mic_device = Some(device);
        settings.transcription_enabled = true;
    }
    if flag("--mic") {
        settings.transcription_enabled = true;
    }
    if let Some(account) = settings::account_id(value("--rank-account").as_deref()) {
        settings.rank_account = Some(account);
    }

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
