//! 設定に従って、辞書・ランク・字幕の処理を動かす。
//!
//! 設定や API キーが変わったら、**入力が変わった処理だけ**を張り直す（ランクの ID を変えても
//! 字幕は止めない）。字幕を止めたときは、確定していない字幕を画面から消す。

use std::path::PathBuf;
use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::settings::{Models, Segmentation, Settings};
use crate::store::Store;
use crate::types::LangCode;
use crate::{assets, audio, rank, secrets, subtitles};

pub struct Options {
    /// 開発・検証用: マイクの代わりに WAV ファイルを流す（設定の開始・停止に関係なく流す）
    pub input_file: Option<PathBuf>,
}

/// 1 つの処理。入力（`K`）が変わったときだけ止めて起動し直す
struct Slot<K> {
    input: Option<K>,
    task: Option<JoinHandle<()>>,
}

impl<K: PartialEq> Slot<K> {
    fn new() -> Self {
        Self {
            input: None,
            task: None,
        }
    }

    /// 入力が変わっていれば止めて `start` し直す
    fn update(&mut self, input: K, start: impl FnOnce() -> Option<JoinHandle<()>>) {
        if self.input.as_ref() == Some(&input) {
            return;
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.input = Some(input);
        self.task = start();
    }
}

#[derive(PartialEq)]
struct AssetsInput {
    languages: Vec<LangCode>,
    openai: Option<String>,
}

#[derive(PartialEq)]
struct RankInput {
    account: Option<u32>,
    languages: Vec<LangCode>,
    poll_seconds: u64,
}

#[derive(PartialEq)]
struct SubtitlesInput {
    /// None なら止めている
    source: Option<Source>,
    mic_language: LangCode,
    translate_to: Vec<LangCode>,
    models: Models,
    delay_ms: u32,
    segmentation: Segmentation,
    mistral: Option<String>,
    openai: Option<String>,
}

#[derive(Clone, PartialEq)]
enum Source {
    Mic(Option<String>),
    File(PathBuf),
}

pub async fn run(store: Arc<Store>, options: Options) {
    let mut settings_rx = store.watch_settings();
    let mut assets_slot = Slot::new();
    let mut rank_slot = Slot::new();
    let mut subtitles_slot = Slot::new();
    loop {
        let settings: Settings = settings_rx.borrow_and_update().clone();
        let keys = tokio::task::spawn_blocking(secrets::load)
            .await
            .unwrap_or_default();
        let languages = settings.glossary_languages();

        assets_slot.update(
            AssetsInput {
                languages: languages.clone(),
                openai: keys.openai.clone(),
            },
            || {
                Some(tokio::spawn(assets::load_into(
                    store.clone(),
                    store.data_dir().join("assets"),
                    keys.openai.clone(),
                )))
            },
        );

        let rank_input = RankInput {
            account: settings.rank_account,
            languages,
            poll_seconds: settings.rank_poll_seconds,
        };
        let account_changed =
            rank_slot.input.as_ref().map(|i: &RankInput| i.account) != Some(settings.rank_account);
        rank_slot.update(rank_input, || {
            // 別のアカウントのランクを出し続けない
            if account_changed {
                store.clear_rank();
            }
            Some(tokio::spawn(rank::run_configured(store.clone())))
        });

        let source = match &options.input_file {
            Some(path) => Some(Source::File(path.clone())),
            None => settings
                .transcription_enabled
                .then(|| Source::Mic(settings.mic_device.clone())),
        };
        let input = SubtitlesInput {
            source: source.clone(),
            mic_language: settings.mic_language.clone(),
            translate_to: settings.translate_to.clone(),
            models: settings.models.clone(),
            delay_ms: settings.target_streaming_delay_ms,
            segmentation: settings.segmentation.clone(),
            mistral: keys.mistral.clone(),
            openai: keys.openai.clone(),
        };
        subtitles_slot.update(input, || {
            store.discard_unsettled();
            start_subtitles(&store, source, keys.mistral, keys.openai)
        });

        if settings_rx.changed().await.is_err() {
            return;
        }
    }
}

fn start_subtitles(
    store: &Arc<Store>,
    source: Option<Source>,
    mistral: Option<String>,
    openai: Option<String>,
) -> Option<JoinHandle<()>> {
    let source = source?;
    let Some(api_key) = mistral else {
        store.set_transcription_error(Some("Mistral の API キーが設定されていません".into()));
        return None;
    };
    store.set_transcription_error(None);
    tracing::info!("transcription started");
    let input = match source {
        Source::Mic(device) => audio::Input::Mic(device),
        Source::File(path) => audio::Input::File(path),
    };
    let store = store.clone();
    Some(tokio::spawn(async move {
        if let Err(e) = subtitles::run(store.clone(), input, api_key, openai).await {
            tracing::error!("transcription stopped: {e:#}");
            store.set_transcription_error(Some(format!("{e:#}")));
        }
    }))
}
