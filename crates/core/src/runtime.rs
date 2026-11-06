//! 設定に従って、辞書・ランク・字幕の処理を動かす。
//!
//! 設定や API キーが変わったら、**入力が変わった処理だけ**を張り直す（ランクの ID を変えても
//! 字幕は止めない）。字幕を止めたときは、確定していない字幕を画面から消す。

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;

use futures_util::FutureExt;
use tokio::task::JoinHandle;

use crate::openai::Access;
use crate::settings::{Models, Region, Segmentation, Settings, SpeechEngine};
use crate::store::Store;
use crate::types::{LangCode, TranscriptionError};
use crate::{assets, audio, rank, secrets, share, subtitles};

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
    access: Option<Access>,
}

#[derive(PartialEq)]
struct RankInput {
    account: Option<u32>,
    languages: Vec<LangCode>,
    poll_seconds: u64,
    region: Option<Region>,
}

#[derive(PartialEq)]
struct SubtitlesInput {
    /// None なら止めている
    source: Option<Source>,
    mic_language: LangCode,
    translate_to: Vec<LangCode>,
    /// 用語ヒントはセッションの開始時にしか渡せないので、変えたら始め直す
    deadlock_mode: bool,
    live_subtitles: bool,
    speech_engine: SpeechEngine,
    models: Models,
    realtime_delay: String,
    segmentation: Segmentation,
    access: Option<Access>,
}

#[derive(Clone, PartialEq)]
enum Source {
    Mic(Option<String>),
    File(PathBuf),
}

pub async fn run(store: Arc<Store>, options: Options) {
    // 依存が ring と aws-lc の両方を有効にするので、rustls は TLS の実装を自分では選べない。
    // 選ばないと、Realtime につなぐときに panic する（reqwest と同じ aws-lc にする）
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let meter = store.meter();
    tokio::spawn(async move { meter.persist().await });
    let mut settings_rx = store.watch_settings();
    let mut assets_slot = Slot::new();
    let mut rank_slot = Slot::new();
    let mut subtitles_slot = Slot::new();
    let mut rank_account = None;
    // 字幕を出していたか（止めた状態から始めるときだけギフトコードの期限を確かめる）
    let mut running = false;
    loop {
        let settings: Settings = settings_rx.borrow_and_update().clone();
        let keys = tokio::task::spawn_blocking(secrets::load)
            .await
            .unwrap_or_default();
        let languages = settings.glossary_languages();
        // 無料のモードは OpenAI に何も送らない
        let needs_key = settings.speech_engine.needs_key();
        let access = keys.access(&settings.api_base).filter(|_| needs_key);

        assets_slot
            .update(
                AssetsInput {
                    languages: languages.clone(),
                    access: access.clone(),
                },
                || {
                    Some(tokio::spawn(assets::load_into(
                        store.clone(),
                        store.data_dir().join("assets"),
                        access.clone(),
                    )))
                },
            )
            .await;

        let rank_input = RankInput {
            account: settings.rank_account,
            languages,
            poll_seconds: settings.rank_poll_seconds,
            region: settings.rank_region,
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
            deadlock_mode: settings.deadlock_mode,
            live_subtitles: settings.wants_live_subtitles(),
            speech_engine: settings.speech_engine,
            models: settings.models.clone(),
            realtime_delay: settings.realtime_delay.clone(),
            segmentation: settings.segmentation.clone(),
            access: access.clone(),
        };
        let starting = !running;
        running = source.is_some();
        subtitles_slot
            .update(input, || {
                store.discard_unsettled();
                let check = starting.then(|| settings.api_base.clone());
                start_subtitles(&store, source, access, needs_key, check)
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
    access: Option<Access>,
    needs_key: bool,
    // Some なら、始める前にギフトコードの期限を確かめる（その API の根元）
    check_gift: Option<String>,
) -> Option<JoinHandle<()>> {
    let Some(source) = source else {
        store.set_transcription_error(None);
        return None;
    };
    if needs_key && access.is_none() {
        store.set_transcription_error(Some(TranscriptionError::MissingKey));
        return None;
    }
    store.set_transcription_error(None);
    tracing::info!("transcription started");
    let (input, ended) = match source {
        Source::Mic(device) => (
            audio::Input::Mic(device),
            Some(TranscriptionError::InputEnded),
        ),
        Source::File(path) => (audio::Input::File(path), None),
    };
    let store = store.clone();
    Some(tokio::spawn(async move {
        // 止まったら理由を出す（開始・停止を押し直すと始め直す）。想定外のエラー（panic）でも
        // 黙って止まらないよう、捕まえて知らせる
        // ギフトコードの期限は始めるときだけ確かめる（配信の途中で設定を変えて作り直すときは
        // 確かめない。止めない。DESIGN.md 17.1）
        if let (Some(access), Some(api)) = (access.as_ref().filter(|a| a.is_gift()), &check_gift)
            && let Err(e) = share::check_gift(api, access.token()).await
        {
            store.set_transcription_error(Some(e));
            return;
        }
        let run = AssertUnwindSafe(subtitles::run(store.clone(), input, access));
        let error = match run.catch_unwind().await {
            Ok(Ok(())) => ended,
            Ok(Err(e)) => Some(TranscriptionError::from_error(&e)),
            Err(panic) => {
                let reason = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown");
                tracing::error!("transcription panicked: {reason}");
                Some(TranscriptionError::Crashed {
                    detail: reason.to_owned(),
                })
            }
        };
        tracing::info!("transcription ended: {error:?}");
        store.set_transcription_error(error);
    }))
}
