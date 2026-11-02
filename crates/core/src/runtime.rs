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

    /// 入力が変わっていれば止めて `start` し直す。前の処理が止まりきってから始める
    /// （止まる途中の処理が、張り直した後の状態を書き換えないように）
    async fn update(&mut self, input: K, start: impl FnOnce() -> Option<JoinHandle<()>>) {
        if self.input.as_ref() == Some(&input) {
            return;
        }
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
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
    let mut rank_account = None;
    loop {
        let settings: Settings = settings_rx.borrow_and_update().clone();
        let keys = tokio::task::spawn_blocking(secrets::load)
            .await
            .unwrap_or_default();
        let languages = settings.glossary_languages();

        assets_slot
            .update(
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
            )
            .await;

        let rank_input = RankInput {
            account: settings.rank_account,
            languages,
            poll_seconds: settings.rank_poll_seconds,
        };
        rank_slot
            .update(rank_input, || {
                // 別のアカウントのランクを出し続けない（前の処理が止まってから消す）
                if std::mem::replace(&mut rank_account, settings.rank_account)
                    != settings.rank_account
                {
                    store.clear_rank();
                }
                Some(tokio::spawn(rank::run_configured(store.clone())))
            })
            .await;

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
        subtitles_slot
            .update(input, || {
                store.discard_unsettled();
                start_subtitles(&store, source, keys.mistral, keys.openai)
            })
            .await;

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
    let Some(source) = source else {
        store.set_transcription_error(None);
        return None;
    };
    let Some(api_key) = mistral else {
        store.set_transcription_error(Some("Mistral の API キーが設定されていません".into()));
        return None;
    };
    store.set_transcription_error(None);
    tracing::info!("transcription started");
    let (input, ended) = match source {
        Source::Mic(device) => (
            audio::Input::Mic(device),
            Some("マイクの入力が途切れました"),
        ),
        Source::File(path) => (audio::Input::File(path), None),
    };
    let store = store.clone();
    Some(tokio::spawn(async move {
        // 止まったら理由を出す（開始・停止を押し直すと始め直す）
        let error = match subtitles::run(store.clone(), input, api_key, openai).await {
            Ok(()) => ended.map(String::from),
            Err(e) => Some(format!("{e:#}")),
        };
        tracing::info!("transcription ended: {error:?}");
        store.set_transcription_error(error);
    }))
}
