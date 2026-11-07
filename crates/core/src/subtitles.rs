//! 字幕パイプライン（DESIGN.md 7 章）。
//!
//! マイク → VAD で区切る ─┬→ GPT-Live-Transcribe の delta を Segment に追記（partial）。
//!                        │   Segment を開いたら音声を送り始め、閉じたら commit する
//!                        └→ Segment が閉じたら区間を切り出して GPT-Transcribe（final）
//!                             → ふりがな・翻訳・固有名詞（enriched）
//!
//! Chrome のモードの経路は `chrome.rs` にある。発話の形とオーバーレイへの出力だけを共有する。
//!
//! Segment の状態を書き換えるのは `Pipeline` だけにする。バッチや enrich の結果もチャンネルで受け取って反映する。

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::annotate::Annotator;
use crate::audio::{self, History, SAMPLE_RATE};
use crate::chrome;
use crate::enrich::Enricher;
use crate::lang;
use crate::now_ms;
use crate::openai::Access;
use crate::settings::{Settings, TextSource};
use crate::store::{ChromeEvent, Store};
use crate::stt::{self, BatchRequest, Hints, RealtimeConfig};
use crate::types::{
    ChromeCommand, Enriched, EntityKind, Glossary, Heard, LangCode, Segment, Stage, Token,
    TranscriptionError, Translated, Translations,
};
use crate::usage::Meter;

/// earshot が受け取るフレームの長さ（16ms）
const FRAME: usize = 256;
const FRAME_MS: u32 = 16;
/// 発話の前後に付ける余白
const PADDING: u64 = SAMPLE_RATE as u64 * 300 / 1000;
/// これより短い発話（咳やクリック）はバッチに送らず捨てる
const MIN_SPEECH_MS: u32 = 250;
/// enrich に文脈として渡す直前の確定字幕の数
const CONTEXT_LINES: usize = 3;
/// 用語ヒントの上限。辞書が増えても送りすぎない
const MAX_KEYWORDS: usize = 500;
/// 音声認識に渡す録音の説明
const PROMPT: &str = "Live commentary by a streamer playing the video game Deadlock (Valve).";
/// デッドロックモードを切ったとき（雑談の配信など）
const PLAIN_PROMPT: &str = "Live commentary by a streamer.";

/// `access` は OpenAI（文字起こし・翻訳）の送り先と鍵
pub async fn run(store: Arc<Store>, input: audio::Input, access: Access) -> anyhow::Result<()> {
    let settings = store.settings();

    // 用語ヒントは Realtime のセッション開始時に 1 回しか渡せないので、辞書を少し待つ。
    // 入力は待ち終わってから開く（待っている間の音声を溜めない）
    let keywords = if settings.deadlock_mode {
        let glossary = store
            .wait_glossary(&settings.mic_language, Duration::from_secs(20))
            .await;
        match &glossary {
            Some(loaded) => keywords(&loaded.glossary, &settings.mic_language),
            None => {
                tracing::warn!("starting transcription without glossary");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    tracing::info!("keywords: {} terms", keywords.len());
    let hints = Arc::new(Hints {
        language: lang::find(&settings.mic_language)
            .map_or_else(|| settings.mic_language.clone(), |l| l.stt.to_owned()),
        prompt: if settings.deadlock_mode {
            PROMPT
        } else {
            PLAIN_PROMPT
        }
        .to_owned(),
        keywords,
    });
    // 文を受け取ったら読みと用語を付ける。ハイブリッドの途中の文は Chrome から届く
    let mut chrome_events = store.subscribe_chrome();
    let annotator = Annotator::new(store.clone(), &settings).await;
    let mut audio_rx = audio::open(input).await?;

    let (done_tx, mut done_rx) = mpsc::channel(64);
    // 開く・音声・閉じるは同じチャンネルで順番どおりに送る
    let (delta_tx, mut delta_rx) = mpsc::channel(256);
    // オフなら Realtime につながず、確定の字幕だけを出す
    let plan = settings.plan();
    let realtime = if plan.partial.text == Some(TextSource::GptLiveTranscribe) {
        let (tx, rx) = mpsc::unbounded_channel();
        let config = RealtimeConfig {
            access: access.clone(),
            model: settings.models.realtime.clone(),
            delay: settings.realtime_delay.clone(),
            meter: store.meter(),
        };
        tokio::spawn(realtime_loop(
            config,
            hints.clone(),
            rx,
            delta_tx,
            done_tx.clone(),
        ));
        Some(tx)
    } else {
        None
    };

    let cloud = Cloud {
        enricher: Arc::new(Enricher::new(
            access.clone(),
            &settings.models,
            settings.mic_language.clone(),
            settings.translate_to.clone(),
            settings.deadlock_mode,
            store.meter(),
        )),
        batch: Arc::new(BatchContext {
            http: crate::http().clone(),
            access,
            model: settings.models.batch.clone(),
            hints,
            meter: store.meter(),
        }),
    };
    let mut pipeline = Pipeline::new(store, &settings, cloud, done_tx, realtime, annotator);

    let mut level = LevelMeter::default();
    while pipeline.refused.is_none() {
        tokio::select! {
            chunk = audio_rx.recv() => {
                let Some(chunk) = chunk else { break };
                if let Some(db) = level.push(&chunk) {
                    pipeline.store.send_level(db);
                }
                pipeline.on_audio(&chunk);
            }
            Some(delta) = delta_rx.recv() => pipeline.on_delta(delta),
            Some(done) = done_rx.recv() => pipeline.on_done(done),
            Ok(event) = chrome_events.recv() => pipeline.on_chrome(event),
        }
    }

    // 入力が終わったら、開いている発話を閉じ、バッチと enrich の結果がすべて届くまで待つ
    pipeline.finish();
    while pipeline.pending > 0 {
        tokio::select! {
            Some(done) = done_rx.recv() => pipeline.on_done(done),
            Some(delta) = delta_rx.recv() => pipeline.on_delta(delta),
        }
    }
    pipeline
        .refused
        .map_or(Ok(()), |refused| Err(refused.into()))
}

/// 操作パネルのメーター用。区間ごとのピークを dBFS で返す
#[derive(Default)]
struct LevelMeter {
    peak: f32,
    samples: usize,
}

impl LevelMeter {
    /// 0.1 秒
    const WINDOW: usize = audio::SAMPLE_RATE as usize / 10;
    const FLOOR_DB: f32 = -90.0;

    fn push(&mut self, chunk: &[f32]) -> Option<f32> {
        self.peak = chunk.iter().fold(self.peak, |p, s| p.max(s.abs()));
        self.samples += chunk.len();
        if self.samples < Self::WINDOW {
            return None;
        }
        let db = (20.0 * self.peak.log10()).max(Self::FLOOR_DB);
        *self = Self::default();
        Some(db)
    }
}

/// OpenAI の確定の字幕と enrich
struct Cloud {
    batch: Arc<BatchContext>,
    enricher: Arc<Enricher>,
}

struct Pipeline {
    store: Arc<Store>,
    lang: LangCode,
    cloud: Cloud,
    done_tx: mpsc::Sender<Done>,
    history: History,
    detector: earshot::Detector,
    segmenter: Segmenter,
    /// リアルタイムの音声認識へ（途中の字幕を出さないときと、入力が終わったあとは None）
    realtime: Option<mpsc::UnboundedSender<stt::Input>>,
    annotator: Annotator,
    live: HashMap<String, Segment>,
    current: Option<String>,
    frame: Vec<f32>,
    frame_pos: u64,
    seq: u64,
    /// 直前の確定字幕（enrich の文脈）
    context: VecDeque<String>,
    /// バッチか enrich の結果を待っている数
    pending: usize,
    /// OpenAI の鍵やギフトコードが断られた。確定の字幕は発話ごとに送るので、これで気づける
    refused: Option<TranscriptionError>,
}

impl Pipeline {
    fn new(
        store: Arc<Store>,
        settings: &Settings,
        cloud: Cloud,
        done_tx: mpsc::Sender<Done>,
        realtime: Option<mpsc::UnboundedSender<stt::Input>>,
        annotator: Annotator,
    ) -> Self {
        Self {
            store,
            lang: settings.mic_language.clone(),
            cloud,
            done_tx,
            history: History::new(60),
            detector: earshot::Detector::default(),
            segmenter: Segmenter::new(&settings.segmentation),
            realtime,
            annotator,
            live: HashMap::new(),
            current: None,
            frame: Vec::with_capacity(FRAME * 2),
            frame_pos: 0,
            seq: 0,
            context: VecDeque::new(),
            pending: 0,
            refused: None,
        }
    }

    fn on_audio(&mut self, chunk: &[f32]) {
        if self.realtime.is_some() {
            self.send_realtime(stt::Input::Audio(chunk.to_vec()));
        }
        self.history.push(chunk);
        self.frame.extend_from_slice(chunk);
        while self.frame.len() >= FRAME {
            let prob = self.detector.predict_f32(&self.frame[..FRAME]);
            self.frame.drain(..FRAME);
            let event = self.segmenter.push(self.frame_pos, prob);
            self.frame_pos += FRAME as u64;
            match event {
                Some(Event::Open) => self.open(),
                Some(Event::Close(close)) => self.close(close),
                None => {}
            }
        }
    }

    fn finish(&mut self) {
        if let Some(close) = self.segmenter.finish() {
            self.close(close);
        }
        // 閉じると、リアルタイムは commit の確認を待って終わる
        self.realtime = None;
    }

    fn send_realtime(&self, input: stt::Input) {
        if let Some(tx) = &self.realtime {
            let _ = tx.send(input);
        }
    }

    fn open(&mut self) {
        self.seq += 1;
        let segment = partial_segment(self.seq, &self.lang);
        let id = segment.id.clone();
        self.send_realtime(stt::Input::Open(id.clone()));
        // 発話が開く前に Chrome が確定した文は、この発話の文ではない
        self.store.send_chrome_command(ChromeCommand::Open);
        self.live.insert(id.clone(), segment);
        self.current = Some(id);
    }

    fn close(&mut self, close: Close) {
        let Some(id) = self.current.take() else {
            return;
        };
        // 捨てる発話も閉じる（閉じないと、その音声が次の発話に混ざる）
        self.send_realtime(stt::Input::Close);
        // Chrome の文を次の発話に持ち越さない
        self.store.cut_chrome();
        if close.speech_ms < MIN_SPEECH_MS {
            self.discard(&id);
            return;
        }
        // 強制的に切った境界は無音ではないので、余白を付けると隣の発話と言葉が重なる
        let from = if close.cut_start {
            close.start
        } else {
            close.start.saturating_sub(PADDING)
        };
        let to = if close.cut_end {
            close.end
        } else {
            close.end + PADDING
        };
        let samples = self.history.slice(from, to);
        let seconds = samples.len() as f64 / f64::from(audio::SAMPLE_RATE);
        let wav = audio::wav(&samples);
        self.pending += 1;
        tokio::spawn(transcribe(
            self.cloud.batch.clone(),
            id,
            wav,
            seconds,
            self.done_tx.clone(),
        ));
    }

    fn discard(&mut self, id: &str) {
        if let Some(seg) = self.live.remove(id) {
            discard(&self.store, seg);
        }
    }

    /// Chrome の途中の文と訳を、開いている発話に載せる。確定の文は OpenAI が作る
    fn on_chrome(&mut self, event: ChromeEvent) {
        match event {
            ChromeEvent::Heard(heard) => self.on_heard(heard),
            ChromeEvent::Translated(Translated {
                id,
                is_final: false,
                translations,
            }) => {
                if let Some(seg) = self.live.get_mut(&id) {
                    let translation_only = self.store.settings().plan().translation_only;
                    if apply_draft(seg, translations, translation_only) {
                        self.store.upsert_segment(seg.clone());
                    }
                }
            }
            ChromeEvent::Translated(_) | ChromeEvent::Closed => {}
        }
    }

    /// 区切られたあとに届いた、前の発話の文は捨てる
    fn on_heard(&mut self, Heard { text, cut, .. }: Heard) {
        let Some(id) = self.current.clone() else {
            return;
        };
        if cut != self.store.chrome_cuts() || text.is_empty() {
            return;
        }
        let seg = self.live.get_mut(&id);
        let Some(seg) = seg.filter(|s| s.stage == Stage::Partial) else {
            return;
        };
        let phase = self.store.settings().plan().partial;
        let (tokens, translate) = self.annotator.annotate(&text, &phase);
        seg.text = text;
        seg.tokens = phase.marks().then_some(tokens);
        self.store.upsert_segment(seg.clone());
        if phase.translation.is_some() {
            chrome::ask_translation(&self.store, id, translate, false);
        }
    }

    fn on_delta(&mut self, delta: stt::Delta) {
        let seg = self.live.get_mut(&delta.segment);
        if let Some(seg) = seg.filter(|s| s.stage == Stage::Partial) {
            seg.text.push_str(&delta.text);
            let phase = self.store.settings().plan().partial;
            seg.tokens = phase
                .marks()
                .then(|| self.annotator.annotate(&seg.text, &phase).0);
            self.store.upsert_segment(seg.clone());
        }
    }

    fn on_done(&mut self, done: Done) {
        match done {
            Done::Transcribed { id, text } => {
                self.pending -= 1;
                self.on_transcribed(&id, text);
            }
            Done::Refused(refused) => self.refused = Some(refused),
            Done::Draft { id, translations } => self.on_draft(&id, translations),
            Done::Enriched { id, enriched } => {
                self.pending -= 1;
                self.on_enriched(&id, enriched);
            }
        }
    }

    fn on_transcribed(&mut self, id: &str, text: Option<String>) {
        let plan = self.store.settings().plan();
        let Some(seg) = self.live.get_mut(id) else {
            return;
        };
        // バッチが失敗したら Realtime で溜めたテキストを使う（途中の字幕を出さないときは出せない）
        let text = text.unwrap_or_else(|| seg.text.clone());
        if text.trim().is_empty() {
            self.discard(id);
            return;
        }
        // 途中の字幕を出さないときはここで初めて表示する。読む時間と並びはこの時刻から数える
        if seg.text.is_empty() {
            seg.started_at = now_ms();
        }
        seg.stage = Stage::Final;
        seg.text = text.clone();
        // 文と読みを同時に入れ替えるので、途中の字幕のふりがなやアイコンが消える瞬間がない
        seg.tokens = plan
            .partial
            .marks()
            .then(|| self.annotator.annotate(&text, &plan.partial).0);
        self.store.upsert_segment(seg.clone());

        let context: Vec<String> = self.context.iter().cloned().collect();
        self.context.push_back(text.clone());
        if self.context.len() > CONTEXT_LINES {
            self.context.pop_front();
        }
        self.pending += 1;
        // 辞書は enrich のたびに store から取る（後から届いたり、作り直されたりする）。
        // ふりがなは出さない設定なら聞かない（字幕を止めずに切り替えられるので、そのときの設定を見る）
        let glossary = self.store.glossary().map(|l| l.glossary);
        let furigana = plan.finals.ruby.is_some();
        tokio::spawn(enrich(
            self.cloud.enricher.clone(),
            id.to_owned(),
            text,
            context,
            glossary,
            furigana,
            self.done_tx.clone(),
        ));
    }

    fn on_draft(&mut self, id: &str, translations: Translations) {
        if let Some(seg) = self.live.get_mut(id) {
            // 翻訳だけなら届いて初めて表示する。読む時間と並びはこの時刻から数える
            if seg.draft.is_none() && self.store.settings().plan().translation_only {
                seg.started_at = now_ms();
            }
            // 途中の字幕の訳が出ていれば、OpenAI の訳がそれを追い越すまで残す
            let draft = seg.draft.get_or_insert_default();
            for (lang, tokens) in translations {
                if draft
                    .get(&lang)
                    .is_none_or(|old| text_len(old) <= text_len(&tokens))
                {
                    draft.insert(lang, tokens);
                }
            }
            self.store.upsert_segment(seg.clone());
        }
    }

    fn on_enriched(&mut self, id: &str, enriched: Enriched) {
        if let Some(seg) = self.live.remove(id) {
            let translation_only = self.store.settings().plan().translation_only;
            self.store
                .upsert_segment(settled(seg, enriched, translation_only));
        }
    }
}

/// 話し始めた発話
pub(crate) fn partial_segment(seq: u64, lang: &str) -> Segment {
    let started_at = now_ms();
    Segment {
        id: format!("mic-{started_at}-{seq}"),
        started_at,
        stage: Stage::Partial,
        lang: lang.to_owned(),
        text: String::new(),
        enriched: None,
        tokens: None,
        draft: None,
        settled_at: None,
    }
}

/// ページの途中の訳を載せる。確定したあとに届いた途中の訳は使わない。載せたら true
pub(crate) fn apply_draft(
    seg: &mut Segment,
    translations: Translations,
    translation_only: bool,
) -> bool {
    if seg.stage != Stage::Partial || translations.is_empty() {
        return false;
    }
    if seg.draft.is_none() && translation_only {
        seg.started_at = now_ms();
    }
    seg.draft = Some(translations);
    true
}

/// まだ表示していなければ流す必要もない
pub(crate) fn discard(store: &Store, mut seg: Segment) {
    if !seg.text.is_empty() {
        seg.stage = Stage::Discarded;
        store.upsert_segment(seg);
    }
}

/// 字幕を完成させる
pub(crate) fn settled(mut seg: Segment, mut enriched: Enriched, translation_only: bool) -> Segment {
    // 訳文が 1 文字も届かなかったときも、翻訳だけならここで初めて表示する
    let draft = seg.draft.take();
    if draft.is_none() && translation_only {
        seg.started_at = now_ms();
    }
    // 訳文のあとで失敗しても、出ている訳文は消さない
    if enriched.translations.is_empty() {
        enriched.translations = draft.unwrap_or_default();
    }
    seg.stage = Stage::Enriched;
    seg.enriched = Some(enriched);
    seg.settled_at = Some(now_ms());
    seg
}

/// 訳文の長さ。用語は言語ごとの名前が分からないので数えない
fn text_len(tokens: &[Token]) -> usize {
    tokens
        .iter()
        .map(|t| match t {
            Token::Text { text, .. } => text.chars().count(),
            Token::Entity { .. } => 0,
        })
        .sum()
}

async fn realtime_loop(
    cfg: RealtimeConfig,
    hints: Arc<Hints>,
    mut input: mpsc::UnboundedReceiver<stt::Input>,
    deltas: mpsc::Sender<stt::Delta>,
    done: mpsc::Sender<Done>,
) {
    let mut speaking = stt::Speaking::default();
    let mut backoff = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        match stt::realtime(&cfg, &hints, &mut speaking, &mut input, &deltas).await {
            Ok(()) => return,
            Err(e) => {
                tracing::warn!("realtime: {e:#}");
                // 張り直しても断られるだけなので、字幕を止めて知らせる
                if let Some(refused) = e.downcast_ref::<TranscriptionError>() {
                    let _ = done.send(Done::Refused(refused.clone())).await;
                    return;
                }
            }
        }
        if started.elapsed() > Duration::from_secs(30) {
            backoff = Duration::from_secs(1);
        }
        // 切れている間の音声は送らない（今話している発話に古い言葉が混ざる）。開く・閉じるだけ追う
        let wait = tokio::time::sleep(backoff);
        tokio::pin!(wait);
        loop {
            tokio::select! {
                () = &mut wait => break,
                next = input.recv() => match next {
                    Some(next) => speaking.skip(next),
                    None => return,
                },
            }
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

struct BatchContext {
    http: reqwest::Client,
    access: Access,
    model: String,
    hints: Arc<Hints>,
    meter: Arc<Meter>,
}

enum Done {
    /// None ならバッチが失敗した
    Transcribed { id: String, text: Option<String> },
    /// 書きかけの訳文
    Draft {
        id: String,
        translations: Translations,
    },
    /// OpenAI の鍵やギフトコードが断られた
    Refused(TranscriptionError),
    /// 翻訳に失敗しても、固有名詞とふりがなは付いている
    Enriched { id: String, enriched: Enriched },
}

async fn transcribe(
    ctx: Arc<BatchContext>,
    id: String,
    wav: Vec<u8>,
    seconds: f64,
    done: mpsc::Sender<Done>,
) {
    let request = BatchRequest {
        access: &ctx.access,
        model: &ctx.model,
        hints: &ctx.hints,
        wav,
        seconds,
        meter: &ctx.meter,
    };
    let text = match stt::transcribe(&ctx.http, request).await {
        Ok(text) => Some(text),
        Err(e) => {
            tracing::warn!("batch transcription: {e:#}");
            if let Some(refused) = e.downcast_ref::<TranscriptionError>() {
                let _ = done.send(Done::Refused(refused.clone())).await;
            }
            None
        }
    };
    let _ = done.send(Done::Transcribed { id, text }).await;
}

async fn enrich(
    enricher: Arc<Enricher>,
    id: String,
    text: String,
    context: Vec<String>,
    glossary: Option<Arc<Glossary>>,
    furigana: bool,
    done: mpsc::Sender<Done>,
) {
    let started = Instant::now();
    let on_draft = |translations| {
        // 書きかけは次のものが追い越すので、詰まっていたら捨てる
        let _ = done.try_send(Done::Draft {
            id: id.clone(),
            translations,
        });
    };
    let enriched = enricher
        .enrich(&text, &context, glossary.as_deref(), furigana, on_draft)
        .await;
    tracing::debug!("enriched in {:?}", started.elapsed());
    let _ = done.send(Done::Enriched { id, enriched }).await;
}

/// 用語ヒント: 話す言語での名前（DESIGN.md 8.4）。
/// 上限を超えるときはヒーロー → アイテム → スキルの順に残す
pub fn keywords(glossary: &Glossary, lang: &str) -> Vec<String> {
    let names = [EntityKind::Hero, EntityKind::Item, EntityKind::Ability]
        .into_iter()
        .flat_map(|kind| glossary.entities.values().filter(move |e| e.kind == kind))
        .filter_map(|e| e.names.get(lang));
    let mut terms: Vec<String> = Vec::new();
    for term in names {
        if terms.len() == MAX_KEYWORDS {
            break;
        }
        if !terms.contains(term) {
            terms.push(term.clone());
        }
    }
    terms
}

#[derive(Debug, PartialEq)]
enum Event {
    Open,
    Close(Close),
}

/// 位置は入力の先頭からのサンプル数
#[derive(Debug, PartialEq)]
struct Close {
    start: u64,
    end: u64,
    speech_ms: u32,
    /// 前の発話を強制的に切った直後から始まっている
    cut_start: bool,
    /// 発話の途中で強制的に切った
    cut_end: bool,
}

/// VAD の確率から発話の区切りを決める（DESIGN.md 3.1）
struct Segmenter {
    min_silence_frames: u32,
    max_frames: u32,
    open: Option<Open>,
    /// 発話していないときに続いている音声フレームの数
    onset: u32,
    /// 直前に強制的に切った位置
    last_cut: Option<u64>,
}

struct Open {
    start: u64,
    last_speech_end: u64,
    speech_frames: u32,
    silence_frames: u32,
    cut_start: bool,
}

impl Segmenter {
    const THRESHOLD: f32 = 0.5;
    /// この数だけ音声フレームが続いたら発話の始まりとみなす
    const ONSET_FRAMES: u32 = 2;
    /// 長すぎる発話は、この長さの無音で閉じる
    const SOFT_SILENCE_FRAMES: u32 = 160 / FRAME_MS;
    /// それでも閉じなければ、最大長からこれだけ過ぎたところで切る
    const HARD_EXTRA_FRAMES: u32 = 4000 / FRAME_MS;

    fn new(cfg: &crate::settings::Segmentation) -> Self {
        Self {
            min_silence_frames: cfg.min_silence_ms / FRAME_MS,
            max_frames: cfg.max_segment_ms / FRAME_MS,
            open: None,
            onset: 0,
            last_cut: None,
        }
    }

    /// `pos` はこのフレームの先頭位置
    fn push(&mut self, pos: u64, prob: f32) -> Option<Event> {
        let speech = prob >= Self::THRESHOLD;
        let frame_end = pos + FRAME as u64;
        let Some(open) = &mut self.open else {
            self.onset = if speech { self.onset + 1 } else { 0 };
            if self.onset < Self::ONSET_FRAMES {
                return None;
            }
            let start = frame_end - u64::from(self.onset) * FRAME as u64;
            self.onset = 0;
            self.open = Some(Open {
                start,
                last_speech_end: frame_end,
                speech_frames: Self::ONSET_FRAMES,
                silence_frames: 0,
                cut_start: self.last_cut == Some(start),
            });
            return Some(Event::Open);
        };

        if speech {
            open.last_speech_end = frame_end;
            open.speech_frames += 1;
            open.silence_frames = 0;
        } else {
            open.silence_frames += 1;
        }
        let frames = ((frame_end - open.start) / FRAME as u64) as u32;
        let hard = frames >= self.max_frames + Self::HARD_EXTRA_FRAMES;
        let close = open.silence_frames >= self.min_silence_frames
            || (frames >= self.max_frames && open.silence_frames >= Self::SOFT_SILENCE_FRAMES)
            || hard;
        if !close {
            return None;
        }
        let open = self.open.take().expect("checked");
        self.last_cut = hard.then_some(frame_end);
        Some(Event::Close(Close {
            start: open.start,
            end: if hard {
                frame_end
            } else {
                open.last_speech_end
            },
            speech_ms: open.speech_frames * FRAME_MS,
            cut_start: open.cut_start,
            cut_end: hard,
        }))
    }

    /// 入力が終わったときに、開いている発話を閉じる
    fn finish(&mut self) -> Option<Close> {
        let open = self.open.take()?;
        Some(Close {
            start: open.start,
            end: open.last_speech_end,
            speech_ms: open.speech_frames * FRAME_MS,
            cut_start: open.cut_start,
            cut_end: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::Access;
    use crate::settings::Segmentation;
    use crate::share;
    use crate::usage::Meter;
    use axum::http::StatusCode;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

    async fn hybrid_pipeline() -> (
        Pipeline,
        tokio::sync::broadcast::Receiver<crate::types::ServerMsg>,
    ) {
        let settings = Settings {
            speech_engine: crate::settings::SpeechEngine::Hybrid,
            ..Settings::default()
        };
        let store = Store::new(Ok(settings.clone()), std::path::PathBuf::new());
        let messages = store.subscribe();
        let access = Access::gift("code".into(), "http://127.0.0.1:9");
        let cloud = Cloud {
            enricher: Arc::new(Enricher::new(
                access.clone(),
                &settings.models,
                settings.mic_language.clone(),
                settings.translate_to.clone(),
                settings.deadlock_mode,
                store.meter(),
            )),
            batch: Arc::new(BatchContext {
                http: crate::http().clone(),
                access,
                model: settings.models.batch.clone(),
                hints: Arc::new(Hints {
                    language: "ja".into(),
                    prompt: String::new(),
                    keywords: Vec::new(),
                }),
                meter: store.meter(),
            }),
        };
        let (done_tx, _done) = mpsc::channel(1);
        let annotator = Annotator::new(store.clone(), &settings).await;
        let pipeline = Pipeline::new(store, &settings, cloud, done_tx, None, annotator);
        (pipeline, messages)
    }

    fn heard(text: &str, cut: u32) -> crate::types::Heard {
        crate::types::Heard {
            text: text.into(),
            is_final: false,
            cut,
        }
    }

    fn close() -> Close {
        Close {
            start: 0,
            end: 0,
            speech_ms: 0,
            cut_start: false,
            cut_end: false,
        }
    }

    /// 流れた字幕の文
    fn last_text(
        messages: &mut tokio::sync::broadcast::Receiver<crate::types::ServerMsg>,
    ) -> Option<String> {
        let mut last = None;
        while let Ok(message) = messages.try_recv() {
            if let crate::types::ServerMsg::SegmentUpsert { segment } = message {
                last = Some(segment.text);
            }
        }
        last
    }

    #[tokio::test]
    async fn a_late_chrome_text_of_the_previous_utterance_is_dropped() {
        let (mut pipeline, mut messages) = hybrid_pipeline().await;
        pipeline.open();
        pipeline.on_chrome(ChromeEvent::Heard(heard("前の文", 0)));
        assert_eq!(last_text(&mut messages).as_deref(), Some("前の文"));

        // 区切ったあとに、区切る前の文が届く
        pipeline.close(close());
        last_text(&mut messages);
        pipeline.open();
        pipeline.on_chrome(ChromeEvent::Heard(heard("前の文の続き", 0)));
        assert_eq!(last_text(&mut messages), None);
        pipeline.on_chrome(ChromeEvent::Heard(heard("次の文", 1)));
        assert_eq!(last_text(&mut messages).as_deref(), Some("次の文"));
    }

    #[tokio::test]
    async fn the_chrome_text_and_its_tokens_arrive_together() {
        let (mut pipeline, mut messages) = hybrid_pipeline().await;
        pipeline.open();
        pipeline.on_chrome(ChromeEvent::Heard(heard("呪い", 0)));
        let segment = std::iter::from_fn(|| messages.try_recv().ok())
            .find_map(|message| match message {
                crate::types::ServerMsg::SegmentUpsert { segment } => Some(segment),
                _ => None,
            })
            .expect("no segment");
        assert_eq!(segment.text, "呪い");
        assert!(segment.tokens.is_some_and(|tokens| !tokens.is_empty()));
    }

    /// 確定の文字起こしを 1 回送り、起きたことを返す
    async fn transcribe_through(access: Access) -> Vec<Done> {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Arc::new(BatchContext {
            http: crate::http().clone(),
            access,
            model: "gpt-transcribe".into(),
            hints: Arc::new(Hints {
                language: "ja".into(),
                prompt: String::new(),
                keywords: Vec::new(),
            }),
            meter: Arc::new(Meter::new(dir.path().join("usage.json"))),
        });
        let (done_tx, mut done_rx) = mpsc::channel(2);
        transcribe(ctx, "id".into(), vec![0; 44], 1.0, done_tx).await;
        std::iter::from_fn(|| done_rx.try_recv().ok()).collect()
    }

    /// 文字起こしの要求にいつも同じ答えを返すサーバー
    async fn answering(status: StatusCode, body: serde_json::Value) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let answer = move || async move { (status, axum::Json(body)) };
        let router =
            axum::Router::new().route("/openai/audio/transcriptions", axum::routing::post(answer));
        tokio::spawn(async move { axum::serve(listener, router).await });
        api
    }

    #[tokio::test]
    async fn a_refused_key_stops_the_subtitles() {
        for (status, error, expected) in [
            (
                StatusCode::UNAUTHORIZED,
                "unknown",
                TranscriptionError::GiftUnknown,
            ),
            (
                StatusCode::FORBIDDEN,
                "revoked",
                TranscriptionError::GiftRevoked,
            ),
            (
                StatusCode::FORBIDDEN,
                "expired",
                TranscriptionError::GiftExpired { expires_at: None },
            ),
            (
                StatusCode::FORBIDDEN,
                "limit",
                TranscriptionError::GiftLimit,
            ),
        ] {
            let api = answering(status, json!({ "error": error })).await;
            let done = transcribe_through(Access::gift("code".into(), &api)).await;
            assert!(
                matches!(&done[0], Done::Refused(refused) if *refused == expected),
                "{error}"
            );
            assert!(matches!(done[1], Done::Transcribed { text: None, .. }));
        }
    }

    /// 1 つの発話の確定の文が抜けるだけで、字幕は続く
    #[tokio::test]
    async fn a_failure_that_may_pass_does_not_stop_the_subtitles() {
        let rate = json!({ "error": "rate" });
        let unavailable = json!({ "error": { "message": "try again" } });
        for api in [
            answering(StatusCode::TOO_MANY_REQUESTS, rate).await,
            answering(StatusCode::INTERNAL_SERVER_ERROR, unavailable).await,
            "http://127.0.0.1:9".to_owned(),
        ] {
            let done = transcribe_through(Access::gift("code".into(), &api)).await;
            assert!(matches!(done[..], [Done::Transcribed { text: None, .. }]));
        }
    }

    #[tokio::test]
    async fn the_pipeline_remembers_why_it_was_refused() {
        let (mut pipeline, _messages) = hybrid_pipeline().await;
        pipeline.on_done(Done::Refused(TranscriptionError::GiftRevoked));
        assert_eq!(pipeline.refused, Some(TranscriptionError::GiftRevoked));
    }

    /// 字幕の途中で断られたら張り直さず、`Done::Refused` を送って終わる
    async fn stops_on_refusal(access: Access, expected: TranscriptionError) {
        let dir = tempfile::tempdir().unwrap();
        let config = RealtimeConfig {
            access,
            model: "gpt-live-transcribe".into(),
            delay: "low".into(),
            meter: Arc::new(Meter::new(dir.path().join("usage.json"))),
        };
        let hints = Arc::new(Hints {
            language: "ja".into(),
            prompt: String::new(),
            keywords: Vec::new(),
        });
        let (_input, rx) = mpsc::unbounded_channel();
        let (delta_tx, _deltas) = mpsc::channel(1);
        let (done_tx, mut done_rx) = mpsc::channel(1);
        let task = tokio::spawn(realtime_loop(config, hints, rx, delta_tx, done_tx));
        let done = tokio::time::timeout(Duration::from_secs(5), done_rx.recv())
            .await
            .expect("the loop should not keep reconnecting");
        assert!(matches!(done, Some(Done::Refused(refused)) if refused == expected));
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn a_realtime_session_closed_at_the_daily_limit_does_not_reconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            let event = |name: &str| Message::text(json!({ "type": name }).to_string());
            ws.send(event("session.created")).await.unwrap();
            ws.next().await.unwrap().unwrap();
            ws.send(event("session.updated")).await.unwrap();
            // Worker の close（server/src/realtime.ts）
            let frame = CloseFrame {
                code: CloseCode::Policy,
                reason: share::LIMIT.into(),
            };
            ws.close(Some(frame)).await.unwrap();
        });
        let access = Access::gift("code".into(), &api);
        stops_on_refusal(access, TranscriptionError::GiftLimit).await;
    }

    #[tokio::test]
    async fn a_realtime_handshake_refused_at_the_daily_limit_does_not_reconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        // Worker の giftKey の答え
        let refuse = || async {
            (
                StatusCode::FORBIDDEN,
                [(share::REFUSAL_HEADER, share::LIMIT)],
                axum::Json(json!({ "error": "limit" })),
            )
        };
        let router = axum::Router::new().route("/openai/realtime", axum::routing::get(refuse));
        tokio::spawn(async move { axum::serve(listener, router).await });
        let access = Access::gift("code".into(), &api);
        stops_on_refusal(access, TranscriptionError::GiftLimit).await;
    }

    /// Worker の 401 にはヘッダーがない
    #[tokio::test]
    async fn a_realtime_handshake_refused_for_an_unknown_code_does_not_reconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let refuse = || async { StatusCode::UNAUTHORIZED };
        let router = axum::Router::new().route("/openai/realtime", axum::routing::get(refuse));
        tokio::spawn(async move { axum::serve(listener, router).await });
        let access = Access::gift("code".into(), &api);
        stops_on_refusal(access, TranscriptionError::GiftUnknown).await;
    }

    #[test]
    fn level_meter_reports_peak_per_window() {
        let mut meter = LevelMeter::default();
        let half = LevelMeter::WINDOW / 2;
        assert_eq!(meter.push(&vec![0.1; half]), None);
        let db = meter.push(&vec![-0.5; half]).unwrap();
        assert!((db - 20.0 * 0.5f32.log10()).abs() < 1e-3);
        assert_eq!(
            meter.push(&vec![0.0; LevelMeter::WINDOW]),
            Some(LevelMeter::FLOOR_DB)
        );
    }

    fn segmenter() -> Segmenter {
        Segmenter::new(&Segmentation {
            min_silence_ms: 400,
            max_segment_ms: 8000,
        })
    }

    /// 確率の列を流し、起きたイベントを (フレーム番号, イベント) で返す
    fn run(seg: &mut Segmenter, probs: impl IntoIterator<Item = f32>) -> Vec<(usize, Event)> {
        probs
            .into_iter()
            .enumerate()
            .filter_map(|(i, p)| seg.push((i * FRAME) as u64, p).map(|e| (i, e)))
            .collect()
    }

    fn frames(ms: u32, prob: f32) -> impl Iterator<Item = f32> {
        std::iter::repeat_n(prob, (ms / FRAME_MS) as usize)
    }

    fn closes(events: &[(usize, Event)]) -> Vec<&Close> {
        events
            .iter()
            .filter_map(|(_, e)| match e {
                Event::Close(c) => Some(c),
                Event::Open => None,
            })
            .collect()
    }

    #[test]
    fn opens_on_speech_and_closes_after_min_silence() {
        let mut s = segmenter();
        let events = run(
            &mut s,
            frames(160, 0.0)
                .chain(frames(800, 0.9))
                .chain(frames(800, 0.0)),
        );
        let speech_start = (160 / FRAME_MS) as u64 * FRAME as u64;
        let speech_end = speech_start + (800 / FRAME_MS) as u64 * FRAME as u64;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].1, Event::Open);
        assert_eq!(
            events[1].1,
            Event::Close(Close {
                start: speech_start,
                end: speech_end,
                speech_ms: 800,
                cut_start: false,
                cut_end: false,
            })
        );
        // 400ms の無音（25 フレーム）が続いたところで閉じる
        assert_eq!(events[1].0, 10 + 50 + 25 - 1);
    }

    #[test]
    fn short_pauses_do_not_split() {
        let mut s = segmenter();
        let events = run(
            &mut s,
            frames(1000, 0.9)
                .chain(frames(300, 0.0))
                .chain(frames(1000, 0.9))
                .chain(frames(500, 0.0)),
        );
        assert_eq!(events.iter().filter(|(_, e)| *e == Event::Open).count(), 1);
    }

    #[test]
    fn long_speech_closes_at_next_short_pause() {
        let mut s = segmenter();
        let events = run(
            &mut s,
            frames(9000, 0.9)
                .chain(frames(200, 0.0))
                .chain(frames(1000, 0.9)),
        );
        // 8 秒を超えた後の 160ms の無音で閉じ、次の発話で開き直す
        let kinds: Vec<_> = events
            .iter()
            .map(|(_, e)| matches!(e, Event::Open))
            .collect();
        assert_eq!(kinds, [true, false, true]);
        assert!(!closes(&events)[0].cut_end);
    }

    #[test]
    fn endless_speech_is_cut_hard_without_overlapping_padding() {
        let mut s = segmenter();
        let events = run(&mut s, frames(14_000, 0.9).chain(frames(500, 0.0)));
        let closes = closes(&events);
        assert_eq!(events[1].0 as u32 + 1, (8000 + 4000) / FRAME_MS);
        assert!(closes[0].cut_end && !closes[0].cut_start);
        // 切った直後から続く発話は、切った位置から始まる
        assert!(closes[1].cut_start && !closes[1].cut_end);
        assert_eq!(closes[1].start, closes[0].end);
    }

    #[test]
    fn finish_closes_open_speech() {
        let mut s = segmenter();
        run(&mut s, frames(600, 0.9));
        let close = s.finish().unwrap();
        assert_eq!(close.speech_ms, 600 / FRAME_MS * FRAME_MS);
        assert!(s.finish().is_none());
    }

    fn glossary(json: serde_json::Value) -> Glossary {
        serde_json::from_value(serde_json::json!({
            "clientVersion": 1, "languages": ["ja"], "entities": json
        }))
        .unwrap()
    }

    #[test]
    fn keywords_collects_names_in_the_language() {
        let g = glossary(serde_json::json!({
            "ability:8": { "kind": "ability", "className": "b", "names": { "ja": "バレットダンス" } },
            "hero:13": { "kind": "hero", "className": "h", "names": { "ja": "ヘイズ", "en": "Haze" } },
            "item:1": { "kind": "item", "className": "i", "names": { "en": "Extended Magazine" } }
        }));
        assert_eq!(keywords(&g, "ja"), ["ヘイズ", "バレットダンス"]);
    }

    #[test]
    fn keywords_keeps_heroes_and_items_first_when_capped() {
        let mut entities = serde_json::Map::new();
        for i in 0..MAX_KEYWORDS {
            entities.insert(
                format!("ability:{i}"),
                serde_json::json!({ "kind": "ability", "className": "a", "names": { "ja": format!("スキル{i}") } }),
            );
        }
        entities.insert(
            "hero:13".into(),
            serde_json::json!({ "kind": "hero", "className": "h", "names": { "ja": "ヘイズ" } }),
        );
        let terms = keywords(&glossary(entities.into()), "ja");
        assert_eq!(terms.len(), MAX_KEYWORDS);
        assert_eq!(terms[0], "ヘイズ");
    }
}
