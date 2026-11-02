//! 字幕パイプライン（DESIGN.md 7 章）。
//!
//! マイク → VAD で区切る ─┬→ Voxtral Realtime の delta を Segment に追記（partial）
//!                        └→ Segment が閉じたら区間を切り出して Small Transcribe 3（final）
//!                             → ふりがな・翻訳・固有名詞（enriched）
//!
//! Segment の状態を書き換えるのは `Pipeline` だけにする。バッチや enrich の結果もチャンネルで受け取って反映する。

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::audio::{self, History, SAMPLE_RATE};
use crate::enrich::Enricher;
use crate::now_ms;
use crate::settings::Settings;
use crate::store::Store;
use crate::stt::{self, BatchRequest, RealtimeConfig};
use crate::types::{Enriched, EntityKind, Glossary, LangCode, Segment, Stage};

/// earshot が受け取るフレームの長さ（16ms）
const FRAME: usize = 256;
const FRAME_MS: u32 = 16;
/// 発話の前後に付ける余白
const PADDING: u64 = SAMPLE_RATE as u64 * 300 / 1000;
/// これより短い発話（咳やクリック）はバッチに送らず捨てる
const MIN_SPEECH_MS: u32 = 250;
/// enrich に文脈として渡す直前の確定字幕の数
const CONTEXT_LINES: usize = 3;
/// 用語ヒントの上限。音声とコンテキストを共有するので、辞書が増えても送りすぎない
const MAX_CONTEXT_BIAS: usize = 500;

/// `api_key` は Mistral（文字起こし）、`openai_key` は OpenAI（翻訳。なければ翻訳しない）
pub async fn run(
    store: Arc<Store>,
    input: audio::Input,
    api_key: String,
    openai_key: Option<String>,
) -> anyhow::Result<()> {
    let settings = store.settings().clone();

    // 用語ヒントは Realtime のセッション開始時に 1 回しか渡せないので、辞書を少し待つ。
    // 入力は待ち終わってから開く（待っている間の音声を溜めない）
    let glossary = store.wait_glossary(Duration::from_secs(20)).await;
    let context_bias = match &glossary {
        Some(loaded) => context_bias(&loaded.glossary, &settings.mic_language),
        None => {
            tracing::warn!("starting transcription without glossary");
            Vec::new()
        }
    };
    tracing::info!("context_bias: {} terms", context_bias.len());
    let mut audio_rx = audio::open(input).await?;

    let (rt_audio_tx, rt_audio_rx) = mpsc::channel(512);
    let (delta_tx, mut delta_rx) = mpsc::channel(256);
    let realtime = RealtimeConfig {
        api_key: api_key.clone(),
        model: settings.models.realtime.clone(),
        language: settings.mic_language.clone(),
        context_bias: context_bias.clone(),
        target_streaming_delay_ms: settings.target_streaming_delay_ms,
    };
    tokio::spawn(realtime_loop(realtime, rt_audio_rx, delta_tx));

    // 形態素解析の辞書の読み込みは同期処理なので、非同期のワーカーを塞がない
    let enricher = {
        let (key, model) = (openai_key, settings.models.translate.clone());
        let effort = settings.models.reasoning_effort.clone();
        let (lang, targets) = (settings.mic_language.clone(), settings.translate_to.clone());
        tokio::task::spawn_blocking(move || Enricher::new(key, model, effort, lang, targets))
            .await??
    };
    let (done_tx, mut done_rx) = mpsc::channel(64);
    let mut pipeline = Pipeline::new(
        store,
        &settings,
        Arc::new(BatchContext {
            http: crate::http().clone(),
            api_key,
            model: settings.models.batch.clone(),
            language: settings.mic_language.clone(),
            context_bias,
        }),
        Arc::new(enricher),
        done_tx,
    );

    loop {
        tokio::select! {
            chunk = audio_rx.recv() => {
                let Some(chunk) = chunk else { break };
                // Realtime が詰まっている（張り直し中など）ときは捨てる。final はバッチで取れる
                let _ = rt_audio_tx.try_send(chunk.clone());
                pipeline.on_audio(&chunk);
            }
            Some(delta) = delta_rx.recv() => pipeline.on_delta(&delta),
            Some(done) = done_rx.recv() => pipeline.on_done(done),
        }
    }

    // 入力が終わったら、開いている発話を閉じ、バッチと enrich の結果がすべて届くまで待つ
    drop(rt_audio_tx);
    pipeline.finish();
    while pipeline.pending > 0 {
        tokio::select! {
            Some(done) = done_rx.recv() => pipeline.on_done(done),
            Some(delta) = delta_rx.recv() => pipeline.on_delta(&delta),
        }
    }
    Ok(())
}

struct Pipeline {
    store: Arc<Store>,
    lang: LangCode,
    batch: Arc<BatchContext>,
    enricher: Arc<Enricher>,
    done_tx: mpsc::Sender<Done>,
    history: History,
    detector: earshot::Detector,
    segmenter: Segmenter,
    router: Router,
    /// delta はおおよそ streaming delay + 処理時間だけ遅れて届く
    grace: Duration,
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
        batch: Arc<BatchContext>,
        enricher: Arc<Enricher>,
        done_tx: mpsc::Sender<Done>,
    ) -> Self {
        Self {
            store,
            lang: settings.mic_language.clone(),
            batch,
            enricher,
            done_tx,
            history: History::new(60),
            detector: earshot::Detector::default(),
            segmenter: Segmenter::new(&settings.segmentation),
            router: Router::default(),
            grace: Duration::from_millis(u64::from(settings.target_streaming_delay_ms) + 1500),
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
    }

    fn open(&mut self) {
        self.seq += 1;
        let started_at = now_ms();
        let id = format!("mic-{started_at}-{}", self.seq);
        self.router.open(id.clone());
        self.live.insert(
            id.clone(),
            Segment {
                id: id.clone(),
                started_at,
                stage: Stage::Partial,
                lang: self.lang.clone(),
                text: String::new(),
                enriched: None,
                settled_at: None,
            },
        );
        self.current = Some(id);
    }

    fn close(&mut self, close: Close) {
        let Some(id) = self.current.take() else {
            return;
        };
        if close.speech_ms < MIN_SPEECH_MS {
            // delta を受け付けないので、次の発話の delta を横取りしないよう枠ごと外す
            self.router.remove(&id);
            self.discard(&id);
            return;
        }
        self.router.close(&id, Instant::now() + self.grace);
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
        let wav = audio::wav(&self.history.slice(from, to));
        self.pending += 1;
        tokio::spawn(transcribe(
            self.batch.clone(),
            id,
            wav,
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

    fn on_delta(&mut self, delta: &str) {
        let Some(id) = self.router.assign(Instant::now()) else {
            return;
        };
        if let Some(seg) = self.live.get_mut(&id).filter(|s| s.stage == Stage::Partial) {
            seg.text.push_str(delta);
            self.store.upsert_segment(seg.clone());
        }
    }

    fn on_done(&mut self, done: Done) {
        self.pending -= 1;
        match done {
            Done::Transcribed { id, text } => self.on_transcribed(&id, text),
            Done::Enriched { id, enriched } => self.on_enriched(&id, enriched),
        }
    }

    fn on_transcribed(&mut self, id: &str, text: Option<String>) {
        let Some(seg) = self.live.get_mut(id) else {
            return;
        };
        // バッチが失敗したら Realtime で溜めたテキストを使う
        let text = text.unwrap_or_else(|| seg.text.clone());
        if text.trim().is_empty() {
            self.discard(id);
            return;
        }
        seg.stage = Stage::Final;
        seg.text = text.clone();
        self.store.upsert_segment(seg.clone());

        let context: Vec<String> = self.context.iter().cloned().collect();
        self.context.push_back(text.clone());
        if self.context.len() > CONTEXT_LINES {
            self.context.pop_front();
        }
        self.pending += 1;
        // 辞書は enrich のたびに store から取る（後から届いたり、作り直されたりする）
        let glossary = self.store.glossary().map(|l| l.glossary);
        tokio::spawn(enrich(
            self.enricher.clone(),
            id.to_owned(),
            text,
            context,
            glossary,
            self.done_tx.clone(),
        ));
    }

    fn on_enriched(&mut self, id: &str, enriched: Enriched) {
        let Some(mut seg) = self.live.remove(id) else {
            return;
        };
        seg.stage = Stage::Enriched;
        seg.enriched = Some(enriched);
        seg.settled_at = Some(now_ms());
        self.store.upsert_segment(seg);
    }
}

async fn realtime_loop(
    cfg: RealtimeConfig,
    mut audio: mpsc::Receiver<Vec<f32>>,
    deltas: mpsc::Sender<String>,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        match stt::realtime(&cfg, &mut audio, &deltas).await {
            Ok(()) => return,
            Err(e) => tracing::warn!("realtime: {e:#}"),
        }
        if started.elapsed() > Duration::from_secs(30) {
            backoff = Duration::from_secs(1);
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
        // 切れている間に溜まった古い音声は送らない（今話している発話に古い言葉が混ざる）
        while audio.try_recv().is_ok() {}
    }
}

struct BatchContext {
    http: reqwest::Client,
    api_key: String,
    model: String,
    language: LangCode,
    context_bias: Vec<String>,
}

enum Done {
    /// None ならバッチが失敗した
    Transcribed { id: String, text: Option<String> },
    /// 翻訳に失敗しても、固有名詞とふりがなは付いている
    Enriched { id: String, enriched: Enriched },
}

async fn transcribe(ctx: Arc<BatchContext>, id: String, wav: Vec<u8>, done: mpsc::Sender<Done>) {
    let request = BatchRequest {
        api_key: &ctx.api_key,
        model: &ctx.model,
        language: &ctx.language,
        context_bias: &ctx.context_bias,
        wav,
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
    done: mpsc::Sender<Done>,
) {
    let started = Instant::now();
    let enriched = enricher.enrich(&text, &context, glossary.as_deref()).await;
    tracing::debug!("enriched in {:?}", started.elapsed());
    let _ = done.send(Done::Enriched { id, enriched }).await;
}

/// 用語ヒント: 話す言語での名前と略称（DESIGN.md 8.4）。
/// 上限を超えるときはヒーロー → アイテム → スキルの順に残す
pub fn context_bias(glossary: &Glossary, lang: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for kind in [EntityKind::Hero, EntityKind::Item, EntityKind::Ability] {
        for entity in glossary.entities.values().filter(|e| e.kind == kind) {
            let names = entity.names.get(lang).into_iter();
            let aliases = entity.aliases.get(lang).into_iter().flatten();
            for term in names.chain(aliases) {
                if terms.len() == MAX_CONTEXT_BIAS {
                    return terms;
                }
                if !terms.contains(term) {
                    terms.push(term.clone());
                }
            }
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

/// delta にはタイムスタンプがないので、まだ受け付けている Segment のうち最も古いものに割り当てる
#[derive(Default)]
struct Router {
    slots: VecDeque<(String, Option<Instant>)>,
}

impl Router {
    fn open(&mut self, id: String) {
        self.slots.push_back((id, None));
    }

    /// 閉じた後も `until` までは delta を受け付ける
    fn close(&mut self, id: &str, until: Instant) {
        if let Some(slot) = self.slots.iter_mut().find(|(s, _)| s == id) {
            slot.1 = Some(until);
        }
    }

    fn remove(&mut self, id: &str) {
        self.slots.retain(|(s, _)| s != id);
    }

    fn assign(&mut self, now: Instant) -> Option<String> {
        while let Some((_, Some(until))) = self.slots.front() {
            if *until > now {
                break;
            }
            self.slots.pop_front();
        }
        self.slots.front().map(|(id, _)| id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Segmentation;

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

    #[test]
    fn router_moves_on_after_grace() {
        let mut r = Router::default();
        let t0 = Instant::now();
        r.open("a".into());
        assert_eq!(r.assign(t0).as_deref(), Some("a"));
        r.close("a", t0 + Duration::from_secs(1));
        r.open("b".into());
        assert_eq!(r.assign(t0).as_deref(), Some("a"));
        assert_eq!(r.assign(t0 + Duration::from_secs(2)).as_deref(), Some("b"));
    }

    #[test]
    fn removed_slot_does_not_take_deltas() {
        let mut r = Router::default();
        r.open("cough".into());
        r.remove("cough");
        r.open("speech".into());
        assert_eq!(r.assign(Instant::now()).as_deref(), Some("speech"));
    }

    fn glossary(json: serde_json::Value) -> Glossary {
        serde_json::from_value(serde_json::json!({
            "clientVersion": 1, "languages": ["ja"], "entities": json
        }))
        .unwrap()
    }

    #[test]
    fn context_bias_collects_names_and_aliases_once() {
        let g = glossary(serde_json::json!({
            "hero:13": { "kind": "hero", "className": "h", "names": { "ja": "ヘイズ", "en": "Haze" }, "aliases": { "ja": ["ヘイズ"] } },
            "item:1": { "kind": "item", "className": "i", "names": { "ja": "基礎拡張マガジン" }, "aliases": { "ja": ["拡張マガジン"] } }
        }));
        assert_eq!(
            context_bias(&g, "ja"),
            ["ヘイズ", "基礎拡張マガジン", "拡張マガジン"]
        );
    }

    #[test]
    fn context_bias_keeps_heroes_and_items_first_when_capped() {
        let mut entities = serde_json::Map::new();
        for i in 0..MAX_CONTEXT_BIAS {
            entities.insert(
                format!("ability:{i}"),
                serde_json::json!({ "kind": "ability", "className": "a", "names": { "ja": format!("スキル{i}") }, "aliases": {} }),
            );
        }
        entities.insert(
            "hero:13".into(),
            serde_json::json!({ "kind": "hero", "className": "h", "names": { "ja": "ヘイズ" }, "aliases": {} }),
        );
        let terms = context_bias(&glossary(entities.into()), "ja");
        assert_eq!(terms.len(), MAX_CONTEXT_BIAS);
        assert_eq!(terms[0], "ヘイズ");
    }
}
