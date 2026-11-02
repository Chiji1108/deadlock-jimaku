//! 設定に従って、辞書・ランク・字幕の処理を動かす。設定や API キーが変わったら全部張り直す
//! （辞書はキャッシュから読むので、張り直しても数秒以内に戻る）。

use std::path::PathBuf;
use std::sync::Arc;

use tokio::task::JoinSet;

use crate::store::Store;
use crate::{assets, audio, rank, secrets, subtitles};

pub struct Options {
    /// 開発・検証用: マイクの代わりに WAV ファイルを流す（設定の開始・停止に関係なく流す）
    pub input_file: Option<PathBuf>,
}

pub async fn run(store: Arc<Store>, options: Options) {
    let mut settings_rx = store.watch_settings();
    loop {
        let settings = settings_rx.borrow_and_update().clone();
        let keys = tokio::task::spawn_blocking(secrets::load)
            .await
            .unwrap_or_default();

        let mut tasks = JoinSet::new();
        tasks.spawn(assets::load_into(
            store.clone(),
            store.data_dir().join("assets"),
            keys.openai.clone(),
        ));
        if settings.rank_account.is_none() {
            store.clear_rank();
        }
        tasks.spawn(rank::run_configured(store.clone()));

        let input = match &options.input_file {
            Some(path) => Some(audio::Input::File(path.clone())),
            None if settings.transcription_enabled => {
                Some(audio::Input::Mic(settings.mic_device.clone()))
            }
            None => None,
        };
        match (input, keys.mistral) {
            (Some(input), Some(api_key)) => {
                let store = store.clone();
                tasks.spawn(async move {
                    if let Err(e) = subtitles::run(store, input, api_key, keys.openai).await {
                        tracing::error!("transcription stopped: {e:#}");
                    }
                });
            }
            (Some(_), None) => {
                tracing::warn!("the Mistral API key is not set; transcription is off");
            }
            (None, _) => {}
        }

        if settings_rx.changed().await.is_err() {
            return;
        }
        tracing::info!("settings changed; restarting");
        tasks.abort_all();
    }
}
