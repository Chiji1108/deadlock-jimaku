//! 設定に従って、辞書・ランク・字幕の処理を動かす。
//!
//! 設定や API キーが変わったら、**入力が変わった処理だけ**を張り直す（ランクの ID を変えても
//! 字幕は止めない）。字幕を止めたときは、確定していない字幕を画面から消す。

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;

use futures_util::FutureExt;
use tokio::task::JoinHandle;

use crate::access::{Access, Accesses};
use crate::llama::Llama;
use crate::local::{self, Gpu, Speech};
use crate::people::Person;
use crate::reazon::Reazon;
use crate::secrets::{Keys, Provider};
use crate::settings::{Engines, FinalSource, Models, Region, Segmentation, Settings, TextSource};
use crate::store::{Fallback, Store};
use crate::types::{LangCode, TranscriptionError};
use crate::whisper::{self, Whisper};
use crate::{assets, audio, chrome, rank, relay, secrets, subtitles};

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
    /// 用語の別名も作る（用語を Decisions API で確かめるとき）
    aliases: bool,
    /// 人の辞書。変えたら、辞書に足し直す
    people: Vec<Person>,
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
    /// 工程が使う会社の送り先と鍵（鍵のない会社は入っていない）
    accesses: Accesses,
    /// PC の中の AI に使わせる GPU（選んだ名前）。変えたら始め直す
    local_gpu: Option<String>,
    /// PC の中の Haas-ja を立てる
    uses_speech: bool,
    /// PC の中の ReazonSpeech を立てる
    uses_reazon: bool,
    /// PC の中の LLM を立てる
    uses_llama: bool,
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
    /// 工程が使う会社の送り先と鍵。要るのに鍵のない会社があれば始めない
    accesses: Accesses,
    /// PC の中の AI に使わせる GPU（選んだ名前）。変えたら始め直す
    local_gpu: Option<String>,
    /// PC の中の LLM を立てる
    uses_llama: bool,
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
    // 0.7 までの「おすそ分け」が資格情報ストアに置いたものは、もう使わないので消す
    tokio::task::spawn_blocking(secrets::forget_share);
    let meter = store.meter();
    tokio::spawn(async move { meter.persist().await });
    tokio::spawn(assets::watch_versions(store.clone()));
    let mut settings_rx = store.watch_settings();
    let mut assets_slot = Slot::new();
    let mut rank_slot = Slot::new();
    let mut subtitles_slot = Slot::new();
    let mut rank_account = None;
    loop {
        settings_rx.borrow_and_update();
        let settings: Settings = store.settings();
        let keys = tokio::task::spawn_blocking(secrets::load)
            .await
            .unwrap_or_default();
        let languages = settings.glossary_languages();
        // ベータのコードで断られて自分のキーで続けている間は、コードを使わない
        let keys = match store.continues_with() {
            Some(Fallback::OwnKeys) => Keys { beta: None, ..keys },
            _ => keys,
        };
        let accesses = keys.accesses(&settings.engines, &settings.api_base);
        let access = accesses.get(Provider::OpenAi);

        assets_slot
            .update(
                AssetsInput {
                    languages: languages.clone(),
                    access: access.clone(),
                    refreshes: store.glossary_refreshes(),
                    aliases: settings.engines.decides_terms(),
                    people: settings.people.clone(),
                },
                || {
                    Some(tokio::spawn(assets::load_into(
                        store.clone(),
                        store.data_dir().join("assets"),
                        access.clone(),
                        settings.engines.decides_terms(),
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

        let input = subtitles_input(&settings, &options, accesses);
        update_subtitles(&mut subtitles_slot, &store, input, &settings.api_base).await;
        // 設定を変えて字幕の処理を作り直しても、ウィンドウは閉じない
        store.sync_chrome();

        if settings_rx.changed().await.is_err() {
            return;
        }
    }
}

fn subtitles_input(settings: &Settings, options: &Options, accesses: Accesses) -> SubtitlesInput {
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
            accesses,
            local_gpu: settings.local_gpu.clone(),
            uses_llama: settings.uses_llama(),
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
        accesses,
        local_gpu: settings.local_gpu.clone(),
        uses_speech: settings.uses_speech(),
        uses_reazon: settings.uses_reazon(),
        uses_llama: settings.uses_llama(),
    }))
}

/// 動いていない状態から始めるときだけ、コードを確かめる（DESIGN.md 17.2）
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
    // Some なら、始める前にコードを確かめる（その API の根元）
    check_code: Option<String>,
) -> Option<JoinHandle<()>> {
    store.set_transcription_error(None);
    match input {
        SubtitlesInput::Stopped => None,
        SubtitlesInput::Chrome(input) => {
            if let Some(provider) = missing_key(&input.engines, &input.accesses) {
                store.set_transcription_error(Some(TranscriptionError::MissingKey { provider }));
                return None;
            }
            tracing::info!("transcription started");
            let store = store.clone();
            Some(tokio::spawn(async move {
                let (engines, accesses) = (&input.engines, &input.accesses);
                if !code_accepted(&store, engines, accesses, check_code.as_deref()).await {
                    return;
                }
                let run = async {
                    let gpu = local_gpu(&store, input.uses_llama).await;
                    let llama = local_llm(&store, input.uses_llama, gpu.as_ref()).await?;
                    chrome::run(store.clone(), input.accesses, llama).await
                };
                supervise(store.clone(), run, None).await;
            }))
        }
        SubtitlesInput::Audio(input) => {
            if let Some(provider) = missing_key(&input.engines, &input.accesses) {
                store.set_transcription_error(Some(TranscriptionError::MissingKey { provider }));
                return None;
            }
            let accesses = input.accesses;
            let engines = input.engines;
            let uses_local = input.uses_speech;
            let uses_reazon = input.uses_reazon;
            let uses_llama = input.uses_llama;
            let whisper = input.engines.whisper();
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
                if !code_accepted(&store, &engines, &accesses, check_code.as_deref()).await {
                    return;
                }
                let run = async {
                    let any = uses_local || whisper.is_some() || uses_llama;
                    let gpu = local_gpu(&store, any).await;
                    let gpu = gpu.as_ref();
                    // PC の中の文字起こしは、途中の字幕と確定で同じ 1 つを使う
                    let local = match uses_local {
                        true => Some(Arc::new(local_speech(&store, gpu).await?)),
                        false => None,
                    };
                    let whisper = local_whisper(&store, whisper, gpu).await?;
                    let reazon = local_reazon(&store, uses_reazon).await?;
                    let llama = local_llm(&store, uses_llama, gpu).await?;
                    let locals = subtitles::Locals {
                        speech: local,
                        whisper,
                        reazon,
                        llama,
                    };
                    subtitles::run(store.clone(), input, accesses, locals).await
                };
                supervise(store.clone(), run, ended).await;
            }))
        }
    }
}

/// 工程に要るのに、送り先と鍵のない会社（OpenAI を先に知らせる）
fn missing_key(engines: &Engines, accesses: &Accesses) -> Option<Provider> {
    let mut needed = engines.providers().into_iter();
    needed.find(|provider| accesses.get(*provider).is_none())
}

/// PC の中の AI に使わせる GPU（PC の中の AI を立てるとき）。GPU が 2 つ以上あれば、設定で選んだものか、
/// 内蔵でないもの。1 つ以下か、数えられなければ None（実行ファイルに任せる）。
/// 数えるのに speech.cpp を使うので、まだなければ取る
async fn local_gpu(store: &Arc<Store>, uses: bool) -> Option<Gpu> {
    if !uses {
        return None;
    }
    let _preparing = store.preparing();
    let gpus = match local::gpus(&store.data_dir().join("speech")).await {
        Ok(gpus) => gpus,
        Err(e) => {
            tracing::warn!("cannot list GPUs: {e:#}");
            return None;
        }
    };
    let gpu = local::pick(&gpus, store.settings().local_gpu.as_deref()).cloned();
    if let Some(gpu) = &gpu {
        tracing::info!("local AI runs on {} ({})", gpu.name, gpu.id);
    }
    gpu
}

/// PC の中の文字起こしを立てる。初回は speech.cpp とモデルを取るので、待っていることを操作パネルに出す
async fn local_speech(store: &Arc<Store>, gpu: Option<&Gpu>) -> anyhow::Result<Speech> {
    anyhow::ensure!(
        store.settings().mic_language == "ja",
        "Haas-ja only transcribes Japanese"
    );
    let _preparing = store.preparing();
    Speech::start(&store.data_dir().join("speech"), gpu).await
}

/// PC の中の Whisper を立てる（確定の文字起こしに選んであるとき）。初回は whisper.cpp とモデルを取る
async fn local_whisper(
    store: &Arc<Store>,
    size: Option<whisper::Size>,
    gpu: Option<&Gpu>,
) -> anyhow::Result<Option<Arc<Whisper>>> {
    let Some(size) = size else {
        return Ok(None);
    };
    let preparing = store.preparing();
    let progress = |done, total| preparing.downloaded(done, total);
    let whisper = Whisper::start(&store.data_dir().join("whisper"), size, gpu, &progress).await?;
    Ok(Some(Arc::new(whisper)))
}

/// PC の中の ReazonSpeech を立てる（確定の文字起こしに選んであるとき）。初回は補助プログラムとモデル（約 0.6 GB）を取る。
/// 辞書の用語をホットワードとして渡す（立てるときにしか渡せない）
async fn local_reazon(store: &Arc<Store>, uses: bool) -> anyhow::Result<Option<Arc<Reazon>>> {
    if !uses {
        return Ok(None);
    }
    let settings = store.settings();
    anyhow::ensure!(
        settings.mic_language == "ja",
        "ReazonSpeech only transcribes Japanese"
    );
    // 辞書を待つ間も、準備していることを出す
    let preparing = store.preparing();
    let (hotwords, _) = subtitles::terms(store, &settings).await;
    let progress = |done, total| preparing.downloaded(done, total);
    let reazon = Reazon::start(&store.data_dir().join("reazon"), &hotwords, &progress).await?;
    Ok(Some(Arc::new(reazon)))
}

/// PC の中の LLM を立てる（使う工程があるとき）。初回は llama.cpp とモデル（約 5 GB）を取るので、
/// 待っていることを操作パネルに出す
async fn local_llm(
    store: &Arc<Store>,
    uses: bool,
    gpu: Option<&Gpu>,
) -> anyhow::Result<Option<Arc<Llama>>> {
    if !uses {
        return Ok(None);
    }
    let preparing = store.preparing();
    let progress = |done, total| preparing.downloaded(done, total);
    let llama = Llama::start(&store.data_dir().join("llama"), gpu, &progress).await?;
    Ok(Some(Arc::new(llama)))
}

/// サーバーに頼む工程があるなら、`check` の API の根元で、ベータのコードがまだ使えるかを確かめる。
/// 断られたら、字幕を止めずに続ける（`fall_back`）か、理由を出して、false。その仕事を今頼めるかは見ない
/// （止めている仕事は、その工程だけがないまま続ける）
async fn code_accepted(
    store: &Store,
    engines: &Engines,
    accesses: &Accesses,
    check: Option<&str>,
) -> bool {
    if !engines.server() {
        return true;
    }
    // サーバーに頼む工程があるのに、コードがない（画面は、選ぶ前にコードをもらう。ないのは、資格情報ストアから
    // コードが消えたとき）。黙って字幕なしにせず、断られたときと同じに続ける
    let checked = match (accesses.server(), check) {
        (None, _) => Err(TranscriptionError::BetaRefused),
        (Some(server), Some(api)) => relay::check_code(api, server.code()).await,
        (Some(_), None) => Ok(()),
    };
    let Err(refused) = checked else {
        return true;
    };
    if !fall_back(store, &refused).await {
        store.set_transcription_error(Some(refused));
    }
    false
}

/// ベータのコードで断られたら、字幕を止めずに続ける（`Store::fall_back`）。サーバーに頼んでいた工程を自分のキーで
/// 頼むのに要る会社（`Engines::with_own_keys`）すべてに自分のキーがあれば、同じ作り方のまま自分のキーで。なければ Chrome の作り方で
async fn fall_back(store: &Store, refused: &TranscriptionError) -> bool {
    // ほかの理由では続けないので、鍵を読みに行かない
    let beta = matches!(
        refused,
        TranscriptionError::BetaLimit
            | TranscriptionError::BetaRefused
            | TranscriptionError::BetaOutdated
    );
    if !beta {
        return false;
    }
    let keys = tokio::task::spawn_blocking(secrets::load)
        .await
        .unwrap_or_default();
    let own = store.saved().engines.with_own_keys();
    let own_keys = own.providers().iter().all(|p| keys.own.contains_key(p));
    store.fall_back(refused, own_keys)
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
    // ベータのコードで断られたら、止めずに Chrome の作り方で続ける
    if let Some(refused) = &error
        && fall_back(&store, refused).await
    {
        return;
    }
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
            accesses: Accesses::default(),
            local_gpu: None,
            uses_llama: false,
        }))
    }

    /// 偽の Worker の、コードの状態を答える口。答えはコードで決まる
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
        let router = axum::Router::new().route("/v2/status", axum::routing::get(answer));
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
            _ => axum::Json(serde_json::json!({ "tasks": ["translate"] })).into_response(),
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
                engines: Engines::STANDARD,
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

        /// `code` があれば、OpenAI の鍵があり、翻訳をそのコードでサーバーに頼む。なければ、鍵もコードもない
        fn audio(&self, code: Option<&str>, language: &str) -> SubtitlesInput {
            let server = code.map(|code| crate::access::Server::new(&self.api, code.into()));
            let engines = match code {
                Some(_) => Engines {
                    translation: crate::settings::TranslationEngine::Server,
                    ..Engines::STANDARD
                },
                None => Engines::STANDARD,
            };
            SubtitlesInput::Audio(Box::new(AudioInput {
                source: Source::File(self.dir.path().join("silence.wav")),
                mic_language: language.into(),
                translate_to: Vec::new(),
                deadlock_mode: false,
                partial_text: Some(TextSource::Chrome),
                engines,
                models: Settings::default().models,
                realtime_delay: "low".into(),
                segmentation: Settings::default().segmentation,
                accesses: Accesses::new(
                    code.map(|_| Access::at(Provider::OpenAi, "key", &self.api)),
                )
                .with_server(server),
                local_gpu: None,
                uses_speech: false,
                uses_reazon: false,
                uses_llama: false,
            }))
        }

        async fn update(&mut self, input: SubtitlesInput) {
            update_subtitles(&mut self.slot, &self.store, input, &self.api).await;
        }

        /// コードの状態を答える口がこの回数呼ばれ、答えが処理に届くまで待つ
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

    /// コードを確かめられずに止まっている間に設定を変えたら、もう一度確かめる
    /// （ベータのコードが断られたときは、止めずに続ける。`Store::fall_back` のテスト）
    #[tokio::test]
    async fn changing_a_setting_while_stopped_checks_the_code_again() {
        let mut app = App::new().await;
        let input = app.audio(Some("down"), "ja");
        app.update(input).await;
        app.settle(1).await;
        assert!(matches!(
            app.error(),
            Some(TranscriptionError::Failed { .. })
        ));

        let input = app.audio(Some("down"), "en");
        app.update(input).await;
        app.settle(2).await;
        assert_eq!(app.checks(), 2);
        assert!(matches!(
            app.error(),
            Some(TranscriptionError::Failed { .. })
        ));
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

        let input = app.audio(Some("down"), "ja");
        app.update(input).await;
        app.settle(1).await;
        assert!(matches!(
            app.error(),
            Some(TranscriptionError::Failed { .. })
        ));
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

    /// サーバーに頼む工程だけでも（会社の鍵を使わなくても）、始める前にコードを確かめる。コードがないときの続け方
    /// （`fall_back`）は、資格情報ストアを読むので、ここでは確かめない
    #[tokio::test]
    async fn the_code_is_checked_for_the_server_alone() {
        use crate::settings::TranslationEngine;
        let mut app = App::new().await;
        let SubtitlesInput::Chrome(mut input) = chrome() else {
            unreachable!()
        };
        input.engines.translation = TranslationEngine::Server;
        let door = crate::access::Server::new(&app.api, "good".into());
        input.accesses = Accesses::default().with_server(Some(door));
        app.update(SubtitlesInput::Chrome(input)).await;
        app.settle(1).await;
        assert_eq!(app.checks(), 1);
        assert_eq!(app.error(), None);
        assert!(app.slot.is_running());
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
            input.accesses = Accesses::new(input.accesses.iter().cloned().chain(xai));
            SubtitlesInput::Audio(input)
        };
        let missing = |provider| Some(TranscriptionError::MissingKey { provider });

        let input = grok(app.audio(Some("good"), "ja"), None, Engines::STANDARD);
        app.update(input).await;
        assert_eq!(app.error(), missing(Provider::Xai));

        let xai = || Some(Access::key(Provider::Xai, "xai".into()));
        let input = grok(app.audio(None, "ja"), xai(), Engines::STANDARD);
        app.update(input).await;
        assert_eq!(app.error(), missing(Provider::OpenAi));

        // OpenAI を使わなければ OpenAI の鍵は要らない
        let input = grok(app.audio(None, "en"), xai(), Engines::CHROME);
        app.update(input).await;
        assert_eq!(app.error(), None);
        assert!(app.slot.is_running());
    }

    /// 動いている間に断られるようになっても、関連の要求で止まる（DESIGN.md 17.2）
    #[tokio::test]
    async fn changes_while_running_do_not_check_the_code() {
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
        let invalid = TranscriptionError::InvalidKey {
            provider: Provider::OpenAi,
        };
        let refused = {
            let invalid = invalid.clone();
            async move { Err(invalid.into()) }
        };
        supervise(store.clone(), refused, None).await;
        assert_eq!(store.transcription_error(), Some(invalid));
    }
}
