//! 設定に従って、辞書・ランク・字幕の処理を動かす。
//!
//! 設定や API キーが変わったら、**入力が変わった処理だけ**を張り直す（ランクの ID を変えても
//! 字幕は止めない）。字幕を止めたときは、確定していない字幕を画面から消す。

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;

use futures_util::FutureExt;
use tokio::task::JoinHandle;

use crate::access::Access;
use crate::secrets::Provider;
use crate::settings::{Engines, FinalSource, Models, Region, Segmentation, Settings, TextSource};
use crate::store::Store;
use crate::types::{LangCode, TranscriptionError};
use crate::{assets, audio, chrome, rank, secrets, share, subtitles};

pub struct Options {
    /// 開発・検証用: マイクの代わりに WAV ファイルを流す（設定の開始・停止に関係なく流す）。
    /// Chrome のモードでは使わない（音声はページが聞く）
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

    /// エラーや入力の終わりで止まった処理は動いていない
    fn is_running(&self) -> bool {
        self.task.as_ref().is_some_and(|task| !task.is_finished())
    }
}

#[derive(PartialEq)]
struct AssetsInput {
    languages: Vec<LangCode>,
    access: Option<Access>,
    /// 新しい版の辞書があれば増える
    refreshes: u64,
}

#[derive(PartialEq)]
struct RankInput {
    account: Option<u32>,
    languages: Vec<LangCode>,
    poll_seconds: u64,
    region: Option<Region>,
}

/// 字幕の処理の入力。モードごとに、使う設定だけを持つ
#[derive(Clone, PartialEq)]
enum SubtitlesInput {
    Stopped,
    Audio(Box<AudioInput>),
    Chrome(Box<ChromeInput>),
}

#[derive(Clone, PartialEq)]
struct AudioInput {
    source: Source,
    mic_language: LangCode,
    translate_to: Vec<LangCode>,
    /// 用語ヒントはセッションの開始時にしか渡せないので、変えたら始め直す
    deadlock_mode: bool,
    /// 途中の字幕の出どころ。表示の切り替えでは字幕を止めない
    partial_text: Option<TextSource>,
    engines: Engines,
    models: Models,
    realtime_delay: String,
    segmentation: Segmentation,
    access: Option<Access>,
    /// xAI の送り先と鍵（xAI を使う工程がなければ None）
    xai: Option<Access>,
}

/// ページは準備が整い直すたびに設定を読むので、ページが使う設定もここに入れる
#[derive(Clone, PartialEq)]
struct ChromeInput {
    mic_device: Option<String>,
    mic_language: LangCode,
    translate_to: Vec<LangCode>,
    deadlock_mode: bool,
    segmentation: Segmentation,
    /// 確定の仕上げ（翻訳・用語・ふりがな）を何で作るか
    engines: Engines,
    models: Models,
    /// OpenAI を使う工程があるのに鍵がなければ始めない
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
    tokio::spawn(assets::watch_versions(store.clone()));
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
        // 使わない会社には何も送らない
        let access = keys
            .access(Provider::OpenAi, &settings.api_base)
            .filter(|_| settings.engines.uses_openai());
        let xai = keys
            .access(Provider::Xai, &settings.api_base)
            .filter(|_| settings.engines.uses_xai());

        assets_slot
            .update(
                AssetsInput {
                    languages: languages.clone(),
                    access: access.clone(),
                    refreshes: store.glossary_refreshes(),
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

        let input = subtitles_input(&settings, &options, access, xai);
        update_subtitles(&mut subtitles_slot, &store, input, &settings.api_base).await;
        // 設定を変えて字幕の処理を作り直しても、ウィンドウは閉じない
        store.sync_chrome();

        if settings_rx.changed().await.is_err() {
            return;
        }
    }
}

fn subtitles_input(
    settings: &Settings,
    options: &Options,
    access: Option<Access>,
    xai: Option<Access>,
) -> SubtitlesInput {
    if settings.engines.transcription == FinalSource::Chrome {
        if !settings.transcription_enabled {
            return SubtitlesInput::Stopped;
        }
        return SubtitlesInput::Chrome(Box::new(ChromeInput {
            mic_device: settings.mic_device.clone(),
            mic_language: settings.mic_language.clone(),
            translate_to: settings.translate_to.clone(),
            deadlock_mode: settings.deadlock_mode,
            segmentation: settings.segmentation.clone(),
            engines: settings.engines,
            models: settings.models.clone(),
            access,
        }));
    }
    let source = match &options.input_file {
        Some(path) => Source::File(path.clone()),
        None if settings.transcription_enabled => Source::Mic(settings.mic_device.clone()),
        None => return SubtitlesInput::Stopped,
    };
    SubtitlesInput::Audio(Box::new(AudioInput {
        source,
        mic_language: settings.mic_language.clone(),
        translate_to: settings.translate_to.clone(),
        deadlock_mode: settings.deadlock_mode,
        partial_text: settings.plan().partial.text,
        engines: settings.engines,
        models: settings.models.clone(),
        realtime_delay: settings.realtime_delay.clone(),
        segmentation: settings.segmentation.clone(),
        access,
        xai,
    }))
}

/// 動いていない状態から始めるときだけ、ギフトコードを確かめる（DESIGN.md 17.1）
async fn update_subtitles(
    slot: &mut Slot<SubtitlesInput>,
    store: &Arc<Store>,
    input: SubtitlesInput,
    api: &str,
) {
    let check = (!slot.is_running()).then(|| api.to_owned());
    slot.update(input.clone(), || {
        store.discard_unsettled();
        start_subtitles(store, input, check)
    })
    .await;
}

fn start_subtitles(
    store: &Arc<Store>,
    input: SubtitlesInput,
    // Some なら、始める前にギフトコードを確かめる（その API の根元）
    check_gift: Option<String>,
) -> Option<JoinHandle<()>> {
    store.set_transcription_error(None);
    match input {
        SubtitlesInput::Stopped => None,
        SubtitlesInput::Chrome(input) => {
            if input.engines.uses_openai() && input.access.is_none() {
                store.set_transcription_error(Some(TranscriptionError::MissingKey {
                    provider: Provider::OpenAi,
                }));
                return None;
            }
            tracing::info!("transcription started");
            let store = store.clone();
            Some(tokio::spawn(async move {
                if !gift_accepted(&store, input.access.iter(), check_gift.as_deref()).await {
                    return;
                }
                let run = chrome::run(store.clone(), input.access);
                supervise(store, run, None).await;
            }))
        }
        SubtitlesInput::Audio(input) => {
            // 足りない鍵があれば始めない（OpenAI を先に知らせる）
            let missing = [
                (
                    Provider::OpenAi,
                    input.engines.uses_openai() && input.access.is_none(),
                ),
                (
                    Provider::Xai,
                    input.engines.uses_xai() && input.xai.is_none(),
                ),
            ]
            .into_iter()
            .find_map(|(provider, missing)| missing.then_some(provider));
            if let Some(provider) = missing {
                store.set_transcription_error(Some(TranscriptionError::MissingKey { provider }));
                return None;
            }
            // 確定が Chrome でなければ、xAI か OpenAI のどちらかを使う（鍵は上で確かめた）
            let transcriber =
                (input.xai.clone().or_else(|| input.access.clone())).expect("finals without a key");
            let (access, xai) = (input.access, input.xai);
            tracing::info!("transcription started");
            let (input, ended) = match input.source {
                Source::Mic(device) => (
                    audio::Input::Mic(device),
                    Some(TranscriptionError::InputEnded),
                ),
                Source::File(path) => (audio::Input::File(path), None),
            };
            let store = store.clone();
            Some(tokio::spawn(async move {
                if !gift_accepted(&store, access.iter().chain(&xai), check_gift.as_deref()).await {
                    return;
                }
                let run = subtitles::run(store.clone(), input, access, transcriber);
                supervise(store, run, ended).await;
            }))
        }
    }
}

/// ギフトコードを使うなら、`check` の API の根元で、使えるか・使う会社を中継するかを確かめる。
/// 断られたら理由を出して false
async fn gift_accepted(
    store: &Store,
    accesses: impl Iterator<Item = &Access>,
    check: Option<&str>,
) -> bool {
    let gifts: Vec<&Access> = accesses.filter(|a| a.is_gift()).collect();
    let (Some(gift), Some(api)) = (gifts.first(), check) else {
        return true;
    };
    let checked = share::check_gift(api, gift.token())
        .await
        .and_then(
            |providers| match gifts.iter().find(|a| !providers.contains(&a.provider())) {
                Some(a) => Err(TranscriptionError::GiftUnsupported {
                    provider: a.provider(),
                }),
                None => Ok(()),
            },
        );
    match checked {
        Ok(()) => true,
        Err(e) => {
            store.set_transcription_error(Some(e));
            false
        }
    }
}

/// 止まったら理由を出す（開始・停止を押し直すと始め直す）。想定外のエラー（panic）でも
/// 黙って止まらないよう、捕まえて知らせる
async fn supervise(
    store: Arc<Store>,
    run: impl Future<Output = anyhow::Result<()>>,
    ended: Option<TranscriptionError>,
) {
    let error = match AssertUnwindSafe(run).catch_unwind().await {
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
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};

    use super::*;

    /// 確かめの答えが処理に届くまでの待ち時間
    const SETTLE: Duration = Duration::from_millis(300);

    fn chrome() -> SubtitlesInput {
        SubtitlesInput::Chrome(Box::new(ChromeInput {
            mic_device: None,
            mic_language: "ja".into(),
            translate_to: Vec::new(),
            deadlock_mode: false,
            segmentation: Settings::default().segmentation,
            engines: Engines::CHROME,
            models: Settings::default().models,
            access: None,
        }))
    }

    /// 偽の Worker の `/gift`。答えはコードで決まる
    async fn gift_server() -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let answer = move |headers: HeaderMap| {
            let hits = counter.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                let code = headers["authorization"].to_str().unwrap();
                gift_answer(code.trim_start_matches("Bearer "))
            }
        };
        let router = axum::Router::new().route("/gift", axum::routing::get(answer));
        tokio::spawn(async move { axum::serve(listener, router).await });
        (api, hits)
    }

    fn gift_answer(code: &str) -> Response {
        let refused = (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({ "error": "revoked" })),
        );
        match code {
            "revoked" => refused.into_response(),
            "down" => StatusCode::BAD_GATEWAY.into_response(),
            // おすそ分けした人が xAI のキーも預けている
            "both" => {
                axum::Json(serde_json::json!({ "providers": ["openai", "xai"] })).into_response()
            }
            // 前のサーバーは中継する会社を返さない（OpenAI だけ）
            _ => axum::Json(serde_json::json!({})).into_response(),
        }
    }

    /// 無音の WAV を 30 秒流す字幕の処理と、偽の Worker
    struct App {
        store: Arc<Store>,
        slot: Slot<SubtitlesInput>,
        api: String,
        checks: Arc<AtomicUsize>,
        dir: tempfile::TempDir,
    }

    impl App {
        async fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let settings = Settings {
                engines: Engines::HYBRID,
                deadlock_mode: false,
                ..Settings::default()
            };
            let silence = vec![0.0; crate::audio::SAMPLE_RATE as usize * 30];
            let wav = crate::audio::wav(&silence);
            std::fs::write(dir.path().join("silence.wav"), wav).unwrap();
            let (api, checks) = gift_server().await;
            Self {
                store: Store::new(Ok(settings), dir.path().to_owned()),
                slot: Slot::new(),
                api,
                checks,
                dir,
            }
        }

        fn audio(&self, code: Option<&str>, language: &str) -> SubtitlesInput {
            SubtitlesInput::Audio(Box::new(AudioInput {
                source: Source::File(self.dir.path().join("silence.wav")),
                mic_language: language.into(),
                translate_to: Vec::new(),
                deadlock_mode: false,
                partial_text: Some(TextSource::Chrome),
                engines: Engines::HYBRID,
                models: Settings::default().models,
                realtime_delay: "low".into(),
                segmentation: Settings::default().segmentation,
                access: code.map(|code| Access::gift(Provider::OpenAi, code.into(), &self.api)),
                xai: None,
            }))
        }

        async fn update(&mut self, input: SubtitlesInput) {
            update_subtitles(&mut self.slot, &self.store, input, &self.api).await;
        }

        /// `/gift` がこの回数呼ばれ、答えが処理に届くまで待つ
        async fn settle(&self, checks: usize) {
            for _ in 0..500 {
                if self.checks() >= checks {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tokio::time::sleep(SETTLE).await;
        }

        fn checks(&self) -> usize {
            self.checks.load(Ordering::SeqCst)
        }

        fn error(&self) -> Option<TranscriptionError> {
            self.store.transcription_error()
        }
    }

    #[tokio::test]
    async fn changing_a_setting_while_stopped_by_a_refusal_checks_the_gift_again() {
        let mut app = App::new().await;
        let input = app.audio(Some("revoked"), "ja");
        app.update(input).await;
        app.settle(1).await;
        assert_eq!(app.error(), Some(TranscriptionError::GiftRevoked));

        let input = app.audio(Some("revoked"), "en");
        app.update(input).await;
        app.settle(2).await;
        assert_eq!(app.checks(), 2);
        assert_eq!(app.error(), Some(TranscriptionError::GiftRevoked));
        assert!(!app.slot.is_running());
    }

    #[tokio::test]
    async fn a_code_entered_after_a_missing_key_is_checked() {
        let mut app = App::new().await;
        let input = app.audio(None, "ja");
        app.update(input).await;
        assert_eq!(
            app.error(),
            Some(TranscriptionError::MissingKey {
                provider: Provider::OpenAi
            })
        );

        let input = app.audio(Some("revoked"), "ja");
        app.update(input).await;
        app.settle(1).await;
        assert_eq!(app.error(), Some(TranscriptionError::GiftRevoked));
    }

    /// 確定の文を Chrome が作っても、OpenAI を使う工程があって鍵がなければ始めない
    #[tokio::test]
    async fn paid_engines_on_chrome_need_a_key() {
        let mut app = App::new().await;
        let SubtitlesInput::Chrome(mut input) = chrome() else {
            unreachable!()
        };
        input.engines.furigana = crate::settings::RubySource::Llm;
        app.update(SubtitlesInput::Chrome(input)).await;
        assert_eq!(
            app.error(),
            Some(TranscriptionError::MissingKey {
                provider: Provider::OpenAi
            })
        );
    }

    /// Grok の確定には xAI のキーが、ほかの工程が OpenAI ならその鍵も要る
    #[tokio::test]
    async fn grok_needs_an_xai_key_and_openai_for_the_rest() {
        let mut app = App::new().await;
        let grok = |input: SubtitlesInput, xai: Option<Access>, engines: Engines| {
            let SubtitlesInput::Audio(mut input) = input else {
                unreachable!()
            };
            input.engines = Engines {
                transcription: FinalSource::GrokVoiceTranscribe,
                ..engines
            };
            input.xai = xai;
            SubtitlesInput::Audio(input)
        };
        let missing = |provider| Some(TranscriptionError::MissingKey { provider });

        let input = grok(app.audio(Some("good"), "ja"), None, Engines::HYBRID);
        app.update(input).await;
        assert_eq!(app.error(), missing(Provider::Xai));

        let xai = || Some(Access::key(Provider::Xai, "xai".into()));
        let input = grok(app.audio(None, "ja"), xai(), Engines::HYBRID);
        app.update(input).await;
        assert_eq!(app.error(), missing(Provider::OpenAi));

        // OpenAI を使わなければ OpenAI の鍵は要らない
        let input = grok(app.audio(None, "en"), xai(), Engines::CHROME);
        app.update(input).await;
        assert_eq!(app.error(), None);
        assert!(app.slot.is_running());
    }

    /// ギフトコードで Grok を使うには、おすそ分けした人が xAI のキーを預けている必要がある
    #[tokio::test]
    async fn grok_through_a_gift_needs_the_issuer_xai_key() {
        let mut app = App::new().await;
        let grok = |app: &App, code: &str| {
            let SubtitlesInput::Audio(mut input) = app.audio(Some(code), "ja") else {
                unreachable!()
            };
            input.engines.transcription = FinalSource::GrokVoiceTranscribe;
            input.xai = Some(Access::gift(Provider::Xai, code.into(), &app.api));
            SubtitlesInput::Audio(input)
        };

        app.update(grok(&app, "good")).await;
        app.settle(1).await;
        assert_eq!(
            app.error(),
            Some(TranscriptionError::GiftUnsupported {
                provider: Provider::Xai
            })
        );
        assert!(!app.slot.is_running());

        app.update(grok(&app, "both")).await;
        app.settle(2).await;
        assert_eq!(app.error(), None);
        assert!(app.slot.is_running());
    }

    /// 動いている間に断られるようになっても、関連の要求で止まる（DESIGN.md 17.1）
    #[tokio::test]
    async fn changes_while_running_do_not_check_the_gift() {
        let mut app = App::new().await;
        app.update(chrome()).await;
        let changes = [
            app.audio(Some("good"), "ja"),
            app.audio(Some("good"), "en"),
            app.audio(Some("down"), "en"),
        ];
        for input in changes {
            app.update(input).await;
            app.settle(0).await;
            assert_eq!(app.checks(), 0);
            assert_eq!(app.error(), None);
            assert!(app.slot.is_running());
        }
    }

    #[tokio::test]
    async fn an_unreachable_server_keeps_the_subtitles_from_starting() {
        let mut app = App::new().await;
        let input = app.audio(Some("down"), "ja");
        app.update(input).await;
        app.settle(1).await;
        assert!(matches!(
            app.error(),
            Some(TranscriptionError::Failed { .. })
        ));
        assert!(!app.slot.is_running());
    }

    #[tokio::test]
    async fn the_reason_a_key_was_refused_reaches_the_panel() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(Ok(Settings::default()), dir.path().to_owned());
        let refused = async { Err(TranscriptionError::GiftRevoked.into()) };
        supervise(store.clone(), refused, None).await;
        assert_eq!(
            store.transcription_error(),
            Some(TranscriptionError::GiftRevoked)
        );
    }
}
