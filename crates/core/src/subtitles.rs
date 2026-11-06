//! 字幕パイプライン（DESIGN.md 7 章）。
//!
//! マイク → VAD で区切る ─┬→ GPT-Live-Transcribe の delta を Segment に追記（partial）。
//!                        │   Segment を開いたら音声を送り始め、閉じたら commit する
//!                        └→ Segment が閉じたら区間を切り出して GPT-Transcribe（final）
//!                             → ふりがな・翻訳・固有名詞（enriched）
//!
//! Segment の状態を書き換えるのは `Pipeline` だけにする。バッチや enrich の結果もチャンネルで受け取って反映する。

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::audio::{self, History, SAMPLE_RATE};
use crate::enrich::Enricher;
use crate::lang;
use crate::local;
use crate::now_ms;
use crate::openai::Access;
use crate::settings::Settings;
use crate::store::Store;
use crate::stt::{self, BatchRequest, Hints, RealtimeConfig};
use crate::types::{
    Enriched, EntityKind, Glossary, LangCode, LiveText, Segment, Stage, Token, Translations,
};
use crate::usage::Meter;

/// earshot が受け取るフレームの長さ（16ms）
const FRAME: usize = 256;
const FRAME_MS: u32 = 16;
/// 発話の前後に付ける余白
const PADDING: u64 = SAMPLE_RATE as u64 * 300 / 1000;
/// これより短い発話（咳やクリック）はバッチに送らず捨てる
const MIN_SPEECH_MS: u32 = 250;
/// ローカル AI に渡す、発話を検出する前の音声（0.3 秒）
const LOCAL_LEAD_SAMPLES: u64 = SAMPLE_RATE as u64 * 300 / 1000;
/// enrich に文脈として渡す直前の確定字幕の数
const CONTEXT_LINES: usize = 3;
/// 用語ヒントの上限。辞書が増えても送りすぎない
const MAX_KEYWORDS: usize = 500;
/// 音声認識に渡す録音の説明
const PROMPT: &str = "Live commentary by a streamer playing the video game Deadlock (Valve).";
/// デッドロックモードを切ったとき（雑談の配信など）
const PLAIN_PROMPT: &str = "Live commentary by a streamer.";

/// `access` は OpenAI（文字起こし・翻訳）の送り先と鍵。無料のモードは使わない
pub async fn run(
    store: Arc<Store>,
    input: audio::Input,
    access: Option<Access>,
) -> anyhow::Result<()> {
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
    let mut audio_rx = audio::open(input).await?;

    // 開く・音声・閉じるは同じチャンネルで順番どおりに送る
    let (delta_tx, mut delta_rx) = mpsc::channel(256);
    // オフなら Realtime につながず、確定の字幕だけを出す
    let realtime = if let Some(access) = access.as_ref().filter(|_| settings.wants_live_subtitles())
    {
        let (tx, rx) = mpsc::unbounded_channel();
        let config = RealtimeConfig {
            access: access.clone(),
            model: settings.models.realtime.clone(),
            delay: settings.realtime_delay.clone(),
            meter: store.meter(),
        };
        tokio::spawn(realtime_loop(config, hints.clone(), rx, delta_tx));
        Some(tx)
    } else {
        None
    };

    let (done_tx, mut done_rx) = mpsc::channel(64);
    // ローカル AI を使うモードでは、音声を補助プログラムにも渡す
    let (local, local_task) = settings
        .speech_engine
        .uses_local()
        .then(|| {
            let (tx, rx) = mpsc::unbounded_channel();
            let terms = store
                .glossary()
                .filter(|_| settings.deadlock_mode)
                .map(|l| local::terms(&l.glossary))
                .unwrap_or_default();
            let task =
                local::Task::spawn(store.clone(), settings.clone(), terms, done_tx.clone(), rx);
            (tx, task)
        })
        .unzip();
    let cloud = access.map(|access| Cloud {
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
    });
    let mut pipeline = Pipeline::new(
        store, &settings, cloud, done_tx, realtime, local, local_task,
    );

    let mut level = LevelMeter::default();
    let mut live = pipeline.store.watch_live();
    live.mark_unchanged();
    loop {
        tokio::select! {
            Ok(()) = live.changed() => {
                let text = live.borrow_and_update().clone();
                pipeline.on_live(text);
            }
            chunk = audio_rx.recv() => {
                let Some(chunk) = chunk else { break };
                if let Some(db) = level.push(&chunk) {
                    pipeline.store.send_level(db);
                }
                pipeline.on_audio(&chunk);
            }
            Some(delta) = delta_rx.recv() => pipeline.on_delta(delta),
            Some(done) = done_rx.recv() => pipeline.on_done(done),
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
    Ok(())
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
    /// 無料のモードにはない
    cloud: Option<Cloud>,
    done_tx: mpsc::Sender<Done>,
    history: History,
    detector: earshot::Detector,
    segmenter: Segmenter,
    /// リアルタイムの音声認識へ（途中の字幕を出さないときと、入力が終わったあとは None）
    realtime: Option<mpsc::UnboundedSender<stt::Input>>,
    local: Option<mpsc::UnboundedSender<local::Input>>,
    /// 無料のモードで確定の字幕を待っている発話。区切った順
    local_finals: VecDeque<String>,
    /// 前の発話を無音でないところで切った
    cut_mid_speech: bool,
    /// 落とすと補助プログラムが止まる
    _local_task: Option<local::Task>,
    live: HashMap<String, Segment>,
    current: Option<String>,
    frame: Vec<f32>,
    frame_pos: u64,
    seq: u64,
    /// 直前の確定字幕（enrich の文脈）
    context: VecDeque<String>,
    /// バッチか enrich の結果を待っている数
    pending: usize,
}

impl Pipeline {
    fn new(
        store: Arc<Store>,
        settings: &Settings,
        cloud: Option<Cloud>,
        done_tx: mpsc::Sender<Done>,
        realtime: Option<mpsc::UnboundedSender<stt::Input>>,
        local: Option<mpsc::UnboundedSender<local::Input>>,
        local_task: Option<local::Task>,
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
            local,
            local_finals: VecDeque::new(),
            cut_mid_speech: false,
            _local_task: local_task,
            live: HashMap::new(),
            current: None,
            frame: Vec::with_capacity(FRAME * 2),
            frame_pos: 0,
            seq: 0,
            context: VecDeque::new(),
            pending: 0,
        }
    }

    fn on_audio(&mut self, chunk: &[f32]) {
        if self.realtime.is_some() {
            self.send_realtime(stt::Input::Audio(chunk.to_vec()));
        }
        self.history.push(chunk);
        if self.current.is_some() {
            self.send_local(local::Input::Audio(chunk.to_vec()));
        }
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

    fn send_local(&self, input: local::Input) {
        if let Some(tx) = &self.local {
            let _ = tx.send(input);
        }
    }

    fn open(&mut self) {
        // 発話を検出するまでの音声も渡す（最初の言葉を落とさない）。強制的に切った境界は
        // 無音ではないので、前の発話と言葉が重なる。今の音声の残りもここで渡す
        let end = self.frame_pos + self.frame.len() as u64;
        let lead = if self.cut_mid_speech {
            self.frame_pos
        } else {
            self.frame_pos.saturating_sub(LOCAL_LEAD_SAMPLES)
        };
        self.send_local(local::Input::Audio(self.history.slice(lead, end)));
        self.seq += 1;
        let started_at = now_ms();
        let id = format!("mic-{started_at}-{}", self.seq);
        self.send_realtime(stt::Input::Open(id.clone()));
        self.live.insert(
            id.clone(),
            Segment {
                id: id.clone(),
                started_at,
                stage: Stage::Partial,
                lang: self.lang.clone(),
                text: String::new(),
                enriched: None,
                draft_source: None,
                draft: None,
                settled_at: None,
            },
        );
        self.current = Some(id);
    }

    fn close(&mut self, close: Close) {
        let Some(id) = self.current.take() else {
            return;
        };
        // 捨てる発話も閉じる（閉じないと、その音声が次の発話に混ざる）
        self.send_realtime(stt::Input::Close);
        self.send_local(local::Input::Cut);
        self.cut_mid_speech = close.cut_end;
        // 無料のモードは補助プログラムが確定の字幕も作る（捨てる発話の分も返ってくる）
        if self.cloud.is_none() {
            self.local_finals.push_back(id.clone());
            self.pending += 1;
        }
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
        let Some(cloud) = &self.cloud else {
            return;
        };
        let samples = self.history.slice(from, to);
        let seconds = samples.len() as f64 / f64::from(audio::SAMPLE_RATE);
        let wav = audio::wav(&samples);
        self.pending += 1;
        tokio::spawn(transcribe(
            cloud.batch.clone(),
            id,
            wav,
            seconds,
            self.done_tx.clone(),
        ));
    }

    fn discard(&mut self, id: &str) {
        if let Some(mut seg) = self.live.remove(id) {
            // まだ表示していなければ流す必要もない
            if !seg.text.is_empty() {
                seg.stage = Stage::Discarded;
                self.store.upsert_segment(seg);
            }
        }
    }

    /// ローカル AI が認識した途中の文を、開いている発話に載せる
    fn on_live(&mut self, live: LiveText) {
        let seg = self.current.as_ref().and_then(|id| self.live.get_mut(id));
        if let Some(seg) = seg.filter(|s| s.stage == Stage::Partial) {
            seg.text = live.text;
            seg.draft_source = Some(live.tokens);
            if !live.translations.is_empty() {
                if seg.draft.is_none() && self.store.settings().translation_only() {
                    seg.started_at = now_ms();
                }
                seg.draft = Some(live.translations);
            }
            self.store.upsert_segment(seg.clone());
        }
    }

    fn on_delta(&mut self, delta: stt::Delta) {
        let seg = self.live.get_mut(&delta.segment);
        if let Some(seg) = seg.filter(|s| s.stage == Stage::Partial) {
            seg.text.push_str(&delta.text);
            self.store.upsert_segment(seg.clone());
        }
    }

    fn on_done(&mut self, done: Done) {
        match done {
            Done::Transcribed { id, text } => {
                self.pending -= 1;
                self.on_transcribed(&id, text);
            }
            Done::Draft { id, translations } => self.on_draft(&id, translations),
            Done::Marked { id, tokens } => self.on_marked(&id, tokens),
            Done::Local(result) => {
                self.pending -= 1;
                self.on_local_final(result);
            }
            Done::Enriched { id, enriched } => {
                self.pending -= 1;
                self.on_enriched(&id, enriched);
            }
        }
    }

    fn on_transcribed(&mut self, id: &str, text: Option<String>) {
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
        // ハイブリッドは途中の字幕の用語とふりがなを残し、確定の文のものが届いたら入れ替える。
        // 補助プログラムが止まっていたら届かないので、確定の文を出す
        let mark = local::Input::Mark {
            id: id.to_owned(),
            text: text.clone(),
        };
        if !self.local.as_ref().is_some_and(|tx| tx.send(mark).is_ok()) {
            seg.draft_source = None;
        }
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
        let furigana = self.store.settings().wants_furigana();
        let Some(cloud) = &self.cloud else {
            return;
        };
        tokio::spawn(enrich(
            cloud.enricher.clone(),
            id.to_owned(),
            text,
            context,
            glossary,
            furigana,
            self.done_tx.clone(),
        ));
    }

    /// enrich が先に終わっていたら、もう発話がないので何もしない
    fn on_marked(&mut self, id: &str, tokens: Vec<Token>) {
        if let Some(seg) = self.live.get_mut(id) {
            seg.draft_source = Some(tokens);
            self.store.upsert_segment(seg.clone());
        }
    }

    /// 無料のモードの確定の字幕。区切った順に届く
    fn on_local_final(&mut self, result: local::Final) {
        let Some(id) = self.local_finals.pop_front() else {
            return;
        };
        if result.text.is_empty() {
            self.discard(&id);
            return;
        }
        if let Some(seg) = self.live.get_mut(&id) {
            if seg.text.is_empty() {
                seg.started_at = now_ms();
            }
            seg.text = result.text;
            self.on_enriched(
                &id,
                Enriched {
                    source: result.tokens,
                    translations: result.translations,
                },
            );
        }
    }

    fn on_draft(&mut self, id: &str, translations: Translations) {
        if let Some(seg) = self.live.get_mut(id) {
            // 翻訳だけなら届いて初めて表示する。読む時間と並びはこの時刻から数える
            if seg.draft.is_none() && self.store.settings().translation_only() {
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

    fn on_enriched(&mut self, id: &str, mut enriched: Enriched) {
        let Some(mut seg) = self.live.remove(id) else {
            return;
        };
        // 訳文が 1 文字も届かなかったときも、翻訳だけならここで初めて表示する
        let draft = seg.draft.take();
        if draft.is_none() && self.store.settings().translation_only() {
            seg.started_at = now_ms();
        }
        // 訳文のあとで失敗しても、出ている訳文は消さない
        if enriched.translations.is_empty() {
            enriched.translations = draft.unwrap_or_default();
        }
        seg.stage = Stage::Enriched;
        seg.enriched = Some(enriched);
        seg.settled_at = Some(now_ms());
        self.store.upsert_segment(seg);
    }
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
) {
    let mut speaking = stt::Speaking::default();
    let mut backoff = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        match stt::realtime(&cfg, &hints, &mut speaking, &mut input, &deltas).await {
            Ok(()) => return,
            Err(e) => tracing::warn!("realtime: {e:#}"),
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

pub(crate) enum Done {
    /// None ならバッチが失敗した
    Transcribed { id: String, text: Option<String> },
    /// 無料のモードで PC の中で作った確定の字幕
    Local(local::Final),
    /// 書きかけの訳文
    Draft {
        id: String,
        translations: Translations,
    },
    /// ハイブリッドの確定の文に、PC の中で読みと用語を付けたもの
    Marked { id: String, tokens: Vec<Token> },
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
    use crate::settings::Segmentation;

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
