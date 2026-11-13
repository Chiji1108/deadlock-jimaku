//! 字幕パイプライン（DESIGN.md 7 章）。
//!
//! マイク → VAD で区切る ─┬→ GPT-Live-Transcribe の delta を Segment に追記（partial）。
//!                        │   Segment を開いたら音声を送り始め、閉じたら commit する
//!                        └→ Segment が閉じたら区間を切り出して GPT-Transcribe（final）
//!                             → ふりがな・翻訳・固有名詞（enriched。`finish.rs`）
//!
//! 確定の文を Chrome が作るときの経路は `chrome.rs` にある。確定の仕上げ（`Finisher`）、
//! 発話の形とオーバーレイへの出力を共有する。
//!
//! Segment の状態を書き換えるのは `Pipeline` だけにする。バッチや enrich の結果もチャンネルで受け取って反映する。

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc};

use crate::access::Accesses;
use crate::annotate::Annotator;
use crate::audio::{self, History, SAMPLE_RATE};
use crate::chrome;
use crate::finish::{self, Finished, Finisher};
use crate::lang;
use crate::llama::Llama;
use crate::local::Speech;
use crate::now_ms;
use crate::segment::{Close, Event, Segmenter};
use crate::settings::{LiveSource, Settings, TextSource};
use crate::store::{ChromeEvent, Store};
use crate::stt::{self, BatchRequest, Hints, RealtimeConfig, Transcriber};
use crate::types::{
    ChromeCommand, Enriched, EntityKind, Glossary, Heard, LangCode, Segment, Stage, Token,
    TranscriptionError, Translated, Translations,
};
use crate::usage::Meter;
use crate::whisper::Whisper;

/// 発話の前後に付ける余白
const PADDING: u64 = SAMPLE_RATE as u64 * 300 / 1000;
/// これより短い発話（咳やクリック）はバッチに送らず捨てる
const MIN_SPEECH_MS: u32 = 250;
/// enrich に文脈として渡す直前の確定字幕の数
const CONTEXT_LINES: usize = 3;
/// 用語ヒントの上限。辞書が増えても送りすぎない
const MAX_KEYWORDS: usize = 500;
/// 音声認識に渡す録音の説明
pub const PROMPT: &str = "Live commentary by a streamer playing the video game Deadlock (Valve).";
/// デッドロックモードを切ったとき（雑談の配信など）
const PLAIN_PROMPT: &str = "Live commentary by a streamer.";

/// `accesses` は工程が使う会社の送り先と鍵、`transcriber` は確定の文字起こしを頼む相手
pub async fn run(
    store: Arc<Store>,
    input: audio::Input,
    accesses: Accesses,
    local: Option<Arc<Speech>>,
    whisper: Option<Arc<Whisper>>,
    llama: Option<Arc<Llama>>,
) -> anyhow::Result<()> {
    let settings = store.settings();
    // 確定の文字起こしを頼む相手。鍵と PC の中の文字起こしは、始める前に確かめてある
    let engines = settings.engines;
    // PC の中の Whisper は、途中の字幕だけに立ててあることもある
    let whisper = |used: bool| whisper.clone().filter(|_| used).map(Transcriber::Whisper);
    let transcriber = whisper(engines.transcription.whisper().is_some())
        .or_else(|| Transcriber::of(engines.transcription.provider(), &accesses, &local));
    let transcriber = transcriber.expect("finals without a transcriber");

    // 用語ヒントは Realtime のセッション開始時に 1 回しか渡せないので、辞書を少し待つ。
    // 入力は待ち終わってから開く（待っている間の音声を溜めない）
    let (keywords, heroes) = if settings.deadlock_mode {
        let glossary = store
            .wait_glossary(&settings.mic_language, Duration::from_secs(20))
            .await;
        match &glossary {
            Some(loaded) => (
                keywords(&loaded.glossary, &settings.mic_language),
                heroes(&loaded.glossary, &settings.mic_language),
            ),
            None => {
                tracing::warn!("starting transcription without glossary");
                (Vec::new(), Vec::new())
            }
        }
    } else {
        (Vec::new(), Vec::new())
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
        heroes,
    });
    // 文を受け取ったら読みと用語を付ける。ハイブリッドの途中の文は Chrome から届く
    let chrome_events = store.subscribe_chrome();
    let annotator = Annotator::new(store.clone(), &settings).await;
    let audio_rx = audio::open(input).await?;

    let (done_tx, done_rx) = mpsc::channel(64);
    let (finished_tx, finished_rx) = mpsc::channel(64);
    // 開く・音声・閉じるは同じチャンネルで順番どおりに送る
    let (delta_tx, delta_rx) = mpsc::channel(256);
    // オフなら Realtime につながず、確定の字幕だけを出す
    let plan = settings.plan();
    // 途中の字幕をアプリが作るなら（Chrome でなければ）、その会社か、PC の中の文字起こしにつなぐ
    let live = plan.partial.text.filter(|text| *text != TextSource::Chrome);
    let live = live.and_then(|_| {
        whisper(engines.live == LiveSource::Whisper)
            .or_else(|| Transcriber::of(engines.live.provider(), &accesses, &local))
    });
    let realtime = if let Some(transcriber) = live {
        let (tx, rx) = mpsc::unbounded_channel();
        let config = RealtimeConfig {
            transcriber,
            model: settings.models_in_use().realtime,
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
        finisher: Arc::new(Finisher::new(store.clone(), &settings, &accesses, llama)),
        finished_tx,
        batch: Arc::new(BatchContext {
            http: crate::http().clone(),
            transcriber,
            model: settings.models_in_use().batch,
            hints,
            meter: store.meter(),
        }),
    };
    let pipeline = Pipeline::new(store, &settings, cloud, done_tx, realtime, annotator);

    listen(
        pipeline,
        audio_rx,
        delta_rx,
        done_rx,
        finished_rx,
        chrome_events,
    )
    .await
}

/// 入力が終わるか、鍵やギフトコードが断られるまで字幕にする
async fn listen(
    mut pipeline: Pipeline,
    mut audio_rx: mpsc::Receiver<Vec<f32>>,
    mut delta_rx: mpsc::Receiver<stt::Delta>,
    mut done_rx: mpsc::Receiver<Done>,
    mut finished_rx: mpsc::Receiver<Finished>,
    mut chrome_events: broadcast::Receiver<ChromeEvent>,
) -> anyhow::Result<()> {
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
            Some(finished) = finished_rx.recv() => pipeline.on_finished(finished),
            Ok(event) = chrome_events.recv() => pipeline.on_chrome(event),
        }
    }

    // 入力が終わったら、開いている発話を閉じ、バッチと enrich の結果がすべて届くまで待つ
    if pipeline.refused.is_none() {
        pipeline.finish();
    }
    while pipeline.pending > 0 && pipeline.refused.is_none() {
        tokio::select! {
            Some(done) = done_rx.recv() => pipeline.on_done(done),
            Some(finished) = finished_rx.recv() => pipeline.on_finished(finished),
            Some(delta) = delta_rx.recv() => pipeline.on_delta(delta),
        }
    }
    // 断られた鍵へは送らない。待っていた結果は捨てる
    match pipeline.refused {
        Some(refused) => {
            pipeline.store.discard_unsettled();
            Err(refused.into())
        }
        None => Ok(()),
    }
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

/// 確定の字幕（OpenAI か xAI）と、その仕上げ
struct Cloud {
    batch: Arc<BatchContext>,
    finisher: Arc<Finisher>,
    finished_tx: mpsc::Sender<Finished>,
}

struct Pipeline {
    store: Arc<Store>,
    lang: LangCode,
    cloud: Cloud,
    done_tx: mpsc::Sender<Done>,
    history: History,
    segmenter: Segmenter,
    /// リアルタイムの音声認識へ（途中の字幕を出さないときと、入力が終わったあとは None）
    realtime: Option<mpsc::UnboundedSender<stt::Input>>,
    annotator: Annotator,
    live: HashMap<String, Segment>,
    current: Option<String>,
    seq: u64,
    /// 直前の確定字幕（enrich の文脈）
    context: VecDeque<String>,
    /// バッチか enrich の結果を待っている数
    pending: usize,
    /// 鍵やギフトコードが断られた。確定の字幕は発話ごとに送るので、これで気づける
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
            segmenter: Segmenter::new(&settings.segmentation),
            realtime,
            annotator,
            live: HashMap::new(),
            current: None,
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
        for event in self.segmenter.feed(chunk) {
            match event {
                Event::Open => self.open(),
                Event::Close(close) => self.close(close),
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
            ChromeEvent::Translated(_) | ChromeEvent::Decided(_) | ChromeEvent::Closed => {}
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
            if delta.whole {
                seg.text = delta.text;
            } else {
                seg.text.push_str(&delta.text);
            }
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
        }
    }

    fn on_finished(&mut self, finished: Finished) {
        match finished {
            Finished::Draft { id, translations } => self.on_draft(&id, translations),
            Finished::Done { id, enriched } => {
                self.pending -= 1;
                self.on_enriched(&id, enriched);
            }
            Finished::Refused(refused) => self.refused = Some(refused),
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

        let context = push_context(&mut self.context, &text);
        let (tokens, translate) = self.annotator.annotate(&text, &plan.finals);
        self.pending += 1;
        let heard = finish::Heard {
            id: id.to_owned(),
            text,
            context,
            tokens,
        };
        let done = self.cloud.finished_tx.clone();
        self.cloud
            .finisher
            .spawn(heard, translate, plan.finals, done);
    }

    fn on_draft(&mut self, id: &str, translations: Translations) {
        if let Some(seg) = self.live.get_mut(id) {
            let translation_only = self.store.settings().plan().translation_only;
            apply_llm_draft(seg, translations, translation_only);
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

/// LLM の書きかけの訳文を載せる。途中の字幕の訳が出ていれば、LLM の訳がそれを追い越すまで残す
pub(crate) fn apply_llm_draft(
    seg: &mut Segment,
    translations: Translations,
    translation_only: bool,
) {
    // 翻訳だけなら届いて初めて表示する。読む時間と並びはこの時刻から数える
    if seg.draft.is_none() && translation_only {
        seg.started_at = now_ms();
    }
    let draft = seg.draft.get_or_insert_default();
    for (lang, tokens) in translations {
        if draft
            .get(&lang)
            .is_none_or(|old| text_len(old) <= text_len(&tokens))
        {
            draft.insert(lang, tokens);
        }
    }
}

/// 確定した文を文脈に足し、足す前の文脈（LLM に渡す直前の確定字幕）を返す
pub(crate) fn push_context(context: &mut VecDeque<String>, text: &str) -> Vec<String> {
    let before = context.iter().cloned().collect();
    context.push_back(text.to_owned());
    if context.len() > CONTEXT_LINES {
        context.pop_front();
    }
    before
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
    transcriber: Transcriber,
    model: String,
    hints: Arc<Hints>,
    meter: Arc<Meter>,
}

enum Done {
    /// None ならバッチが失敗した
    Transcribed { id: String, text: Option<String> },
    /// 鍵やギフトコードが断られた
    Refused(TranscriptionError),
}

async fn transcribe(
    ctx: Arc<BatchContext>,
    id: String,
    wav: Vec<u8>,
    seconds: f64,
    done: mpsc::Sender<Done>,
) {
    let request = BatchRequest {
        transcriber: &ctx.transcriber,
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

/// ヒーローの名前（話す言語で）。Whisper に渡す用語（DESIGN.md 9.10）
pub fn heroes(glossary: &Glossary, lang: &str) -> Vec<String> {
    let heroes = glossary
        .entities
        .values()
        .filter(|e| e.kind == EntityKind::Hero);
    let mut names: Vec<String> = heroes.filter_map(|e| e.names.get(lang).cloned()).collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::access::Access;
    use crate::secrets::Provider;
    use crate::segment::{FRAME, FRAME_MS};
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
        let (done_tx, _done) = mpsc::channel(1);
        let access = Access::gift(Provider::OpenAi, "code".into(), "http://127.0.0.1:9");
        let (pipeline, _finished) = hybrid_pipeline_to(access, done_tx).await;
        let messages = pipeline.store.subscribe();
        (pipeline, messages)
    }

    async fn hybrid_pipeline_to(
        access: Access,
        done_tx: mpsc::Sender<Done>,
    ) -> (Pipeline, mpsc::Receiver<Finished>) {
        let settings = Settings {
            engines: crate::settings::Engines::STANDARD,
            ..Settings::default()
        };
        let store = Store::new(Ok(settings.clone()), std::path::PathBuf::new());
        let (finished_tx, finished_rx) = mpsc::channel(64);
        let cloud = Cloud {
            finisher: Arc::new(Finisher::new(
                store.clone(),
                &settings,
                &Accesses::new([access.clone()]),
                None,
            )),
            finished_tx,
            batch: Arc::new(BatchContext {
                http: crate::http().clone(),
                transcriber: Transcriber::Api(access),
                model: settings.models.batch.clone(),
                hints: Arc::new(Hints {
                    language: "ja".into(),
                    prompt: String::new(),
                    keywords: Vec::new(),
                    heroes: Vec::new(),
                }),
                meter: store.meter(),
            }),
        };
        let annotator = Annotator::new(store.clone(), &settings).await;
        let pipeline = Pipeline::new(store, &settings, cloud, done_tx, None, annotator);
        (pipeline, finished_rx)
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
        transcribe_with(access, Vec::new()).await
    }

    async fn transcribe_with(transcriber: Access, keywords: Vec<String>) -> Vec<Done> {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Arc::new(BatchContext {
            http: crate::http().clone(),
            transcriber: Transcriber::Api(transcriber),
            model: "gpt-transcribe".into(),
            hints: Arc::new(Hints {
                language: "ja".into(),
                prompt: String::new(),
                keywords,
                heroes: Vec::new(),
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
            let done =
                transcribe_through(Access::gift(Provider::OpenAi, "code".into(), &api)).await;
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
            let done =
                transcribe_through(Access::gift(Provider::OpenAi, "code".into(), &api)).await;
            assert!(matches!(done[..], [Done::Transcribed { text: None, .. }]));
        }
    }

    /// xAI には言語と用語（100 件・50 文字まで）を送り、ファイルを最後に置く
    #[tokio::test]
    async fn grok_gets_the_language_and_up_to_100_keyterms_before_the_file() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let (fields_tx, mut fields_rx) = mpsc::unbounded_channel();
        // multipart の欄を、送られた順に名前と値で読む（ファイルの中身は読まない）
        let answer = move |body: axum::body::Bytes| {
            let fields_tx = fields_tx.clone();
            async move {
                let body = String::from_utf8_lossy(&body);
                let fields: Vec<(String, String)> = body
                    .split(" name=\"")
                    .skip(1)
                    .map(|part| {
                        let (name, rest) = part.split_once('"').unwrap();
                        let value = rest.split("\r\n\r\n").nth(1).unwrap_or_default();
                        let value = value.split("\r\n").next().unwrap_or_default();
                        (name.to_owned(), value.to_owned())
                    })
                    .collect();
                fields_tx.send(fields).unwrap();
                axum::Json(json!({ "text": " ヘイズ強い " }))
            }
        };
        let router = axum::Router::new().route("/xai/stt", axum::routing::post(answer));
        tokio::spawn(async move { axum::serve(listener, router).await });

        let mut keywords = vec!["x".repeat(51)];
        keywords.extend((0..150).map(|i| format!("用語{i}")));
        // ギフトコードでも自分のキーでも、同じ欄を送る（ギフトコードは開発者のサーバーが中継する）
        let transcriber = Access::gift(Provider::Xai, "code".into(), &api);
        let done = transcribe_with(transcriber, keywords).await;
        assert!(
            matches!(&done[..], [Done::Transcribed { text: Some(t), .. }] if t == "ヘイズ強い")
        );
        let fields = fields_rx.recv().await.unwrap();
        let value = |name: &str| {
            fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(value("language"), Some("ja"));
        let keyterms: Vec<_> = fields.iter().filter(|(n, _)| n == "keyterm").collect();
        assert_eq!(keyterms.len(), 100);
        assert_eq!(keyterms[0].1, "用語0");
        assert_eq!(fields.last().unwrap().0, "file");
    }

    /// 間違った鍵には 400 が返る。ほかの 400 では止めない。ギフトコードなら、おすそ分けした人の鍵が断られた
    #[tokio::test]
    async fn a_refused_xai_key_stops_the_subtitles() {
        let incorrect = r#"{"code":"Client specified an invalid argument","error":"Incorrect API key provided. You can obtain an API key from https://console.x.ai."}"#;
        for (status, body, refused) in [
            (StatusCode::BAD_REQUEST, incorrect, true),
            (StatusCode::UNAUTHORIZED, "", true),
            (
                StatusCode::BAD_REQUEST,
                r#"{"error":"unsupported format"}"#,
                false,
            ),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let api = format!("http://{}", listener.local_addr().unwrap());
            let answer = move || async move { (status, body) };
            let router = axum::Router::new().route("/xai/stt", axum::routing::post(answer));
            tokio::spawn(async move { axum::serve(listener, router).await });
            let transcriber = Access::gift(Provider::Xai, "code".into(), &api);
            let done = transcribe_with(transcriber, Vec::new()).await;
            let expected = TranscriptionError::GiftUnknown;
            assert_eq!(
                matches!(&done[0], Done::Refused(r) if *r == expected),
                refused,
                "{status} {body}"
            );
        }
    }

    #[tokio::test]
    async fn the_pipeline_remembers_why_it_was_refused() {
        let (mut pipeline, _messages) = hybrid_pipeline().await;
        pipeline.on_done(Done::Refused(TranscriptionError::GiftRevoked));
        assert_eq!(pipeline.refused, Some(TranscriptionError::GiftRevoked));
    }

    /// 1 回目の発話の文字起こしは答えず、2 回目から断るサーバー。呼ばれた回数も返す
    async fn refusing_after_one_silent_request() -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let answer = move || {
            let requests = counter.clone();
            async move {
                if requests.fetch_add(1, Ordering::SeqCst) == 0 {
                    std::future::pending::<()>().await;
                }
                (
                    StatusCode::FORBIDDEN,
                    axum::Json(json!({ "error": "revoked" })),
                )
            }
        };
        let router =
            axum::Router::new().route("/openai/audio/transcriptions", axum::routing::post(answer));
        tokio::spawn(async move { axum::serve(listener, router).await });
        (api, requests)
    }

    fn utterance(pipeline: &mut Pipeline) {
        pipeline.open();
        utterance_alongside(pipeline);
    }

    /// 開いている発話を残したまま、別の発話を 1 つ文字起こしに送る
    fn utterance_alongside(pipeline: &mut Pipeline) {
        let open = pipeline.current.take();
        pipeline.seq += 1;
        let segment = partial_segment(pipeline.seq, &pipeline.lang);
        pipeline.current = Some(segment.id.clone());
        pipeline.live.insert(segment.id.clone(), segment);
        pipeline.close(Close {
            start: 0,
            end: audio::SAMPLE_RATE.into(),
            speech_ms: 1000,
            cut_start: false,
            cut_end: false,
        });
        pipeline.current = open;
    }

    /// `MIN_SPEECH_MS` を超えて開いたままの発話を、書きかけの文とともに作る
    fn open_utterance(pipeline: &mut Pipeline) -> String {
        let frames = u64::from(MIN_SPEECH_MS / FRAME_MS) + 1;
        for frame in 0..frames {
            if pipeline.segmenter.push(frame * FRAME as u64, 1.0) == Some(Event::Open) {
                pipeline.open();
            }
        }
        let segment = pipeline.current.clone().unwrap();
        pipeline.on_delta(stt::Delta {
            segment: segment.clone(),
            text: "ヘイズ".into(),
            whole: false,
        });
        segment
    }

    /// 答えない要求が残っていても待たず、開いている発話を断られた鍵へ送らず、出ている字幕を消す
    #[tokio::test]
    async fn a_refusal_stops_at_once_and_drops_what_is_left() {
        let (api, requests) = refusing_after_one_silent_request().await;
        let (done_tx, done_rx) = mpsc::channel(64);
        let (mut pipeline, finished_rx) =
            hybrid_pipeline_to(Access::gift(Provider::OpenAi, "code".into(), &api), done_tx).await;
        let store = pipeline.store.clone();
        let mut messages = store.subscribe();
        let open = open_utterance(&mut pipeline);
        utterance_alongside(&mut pipeline);
        utterance_alongside(&mut pipeline);

        let (_audio, audio_rx) = mpsc::channel(1);
        let (_delta, delta_rx) = mpsc::channel(1);
        let events = store.subscribe_chrome();
        let run = listen(pipeline, audio_rx, delta_rx, done_rx, finished_rx, events);
        let refused = tokio::time::timeout(Duration::from_secs(2), run)
            .await
            .expect("the refusal does not wait for the silent request")
            .unwrap_err();
        assert_eq!(
            TranscriptionError::from_error(&refused),
            TranscriptionError::GiftRevoked
        );

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        let discarded = std::iter::from_fn(|| messages.try_recv().ok()).any(|message| {
            matches!(message, crate::types::ServerMsg::SegmentUpsert { segment }
                if segment.id == open && segment.stage == Stage::Discarded)
        });
        assert!(discarded);
    }

    /// 入力が終わって残りを待っている間に断られても、待ち続けない
    #[tokio::test]
    async fn a_refusal_while_draining_stops_at_once() {
        let (api, _requests) = refusing_after_one_silent_request().await;
        let (done_tx, done_rx) = mpsc::channel(64);
        let (mut pipeline, finished_rx) =
            hybrid_pipeline_to(Access::gift(Provider::OpenAi, "code".into(), &api), done_tx).await;
        let events = pipeline.store.subscribe_chrome();
        utterance(&mut pipeline);
        utterance(&mut pipeline);

        let (_, audio_rx) = mpsc::channel(1);
        let (_delta, delta_rx) = mpsc::channel(1);
        let run = listen(pipeline, audio_rx, delta_rx, done_rx, finished_rx, events);
        let refused = tokio::time::timeout(Duration::from_secs(2), run)
            .await
            .expect("the refusal does not wait for the silent request")
            .unwrap_err();
        assert_eq!(
            TranscriptionError::from_error(&refused),
            TranscriptionError::GiftRevoked
        );
    }

    /// 字幕の途中で断られたら張り直さず、`Done::Refused` を送って終わる
    async fn stops_on_refusal(access: Access, expected: TranscriptionError) {
        let dir = tempfile::tempdir().unwrap();
        let config = RealtimeConfig {
            transcriber: Transcriber::Api(access),
            model: "gpt-live-transcribe".into(),
            delay: "low".into(),
            meter: Arc::new(Meter::new(dir.path().join("usage.json"))),
        };
        let hints = Arc::new(Hints {
            language: "ja".into(),
            prompt: String::new(),
            keywords: Vec::new(),
            heroes: Vec::new(),
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
        let access = Access::gift(Provider::OpenAi, "code".into(), &api);
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
        let access = Access::gift(Provider::OpenAi, "code".into(), &api);
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
        let access = Access::gift(Provider::OpenAi, "code".into(), &api);
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
