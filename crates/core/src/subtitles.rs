//! 字幕パイプライン（DESIGN.md 7 章）。
//!
//! マイク → VAD で区切る ─┬→ Voxtral Realtime の delta を Segment に追記（partial）
//!                        └→ Segment が閉じたら区間を切り出して Small Transcribe 3（final）
//!
//! Segment の状態を書き換えるのはこのタスクだけにする。バッチの結果もチャンネルで受け取って反映する。

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::audio::{self, History, SAMPLE_RATE};
use crate::now_ms;
use crate::store::Store;
use crate::stt::{self, BatchRequest, RealtimeConfig};
use crate::types::{Glossary, LangCode, Segment, Stage};

/// earshot が受け取るフレームの長さ（16ms）
const FRAME: usize = 256;
const FRAME_MS: u32 = 16;
/// 発話の前後に付ける余白
const PADDING: u64 = SAMPLE_RATE as u64 * 300 / 1000;
/// これより短い発話（咳やクリック）はバッチに送らず捨てる
const MIN_SPEECH_MS: u32 = 250;

pub async fn run(store: Arc<Store>, input: audio::Input, api_key: String) -> anyhow::Result<()> {
    let settings = store.settings().clone();
    let mut audio_rx = audio::open(input)?;

    // 用語ヒントは Realtime のセッション開始時に 1 回しか渡せないので、辞書を少し待つ
    let context_bias = match store.wait_glossary(Duration::from_secs(20)).await {
        Some(loaded) => context_bias(&loaded.glossary, &settings.mic_language),
        None => {
            tracing::warn!("starting transcription without glossary");
            Vec::new()
        }
    };
    tracing::info!("context_bias: {} terms", context_bias.len());

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

    let batch = Arc::new(BatchContext {
        http: reqwest::Client::new(),
        api_key,
        model: settings.models.batch.clone(),
        language: settings.mic_language.clone(),
        context_bias,
    });
    let (final_tx, mut final_rx) = mpsc::channel::<Finalized>(64);

    let mut history = History::new(60);
    let mut detector = earshot::Detector::default();
    let mut segmenter = Segmenter::new(&settings.segmentation);
    // delta はおおよそ streaming delay + 処理時間だけ遅れて届く
    let grace = Duration::from_millis(u64::from(settings.target_streaming_delay_ms) + 1500);
    let mut router = Router::default();
    let mut live: HashMap<String, Segment> = HashMap::new();
    let mut frame: Vec<f32> = Vec::with_capacity(FRAME * 2);
    let mut frame_pos: u64 = 0;
    let mut seq: u64 = 0;
    let mut current: Option<String> = None;

    loop {
        tokio::select! {
            chunk = audio_rx.recv() => {
                let Some(chunk) = chunk else { break };
                history.push(&chunk);
                // Realtime が詰まっている（張り直し中など）ときは捨てる。final はバッチで取れる
                let _ = rt_audio_tx.try_send(chunk.clone());
                frame.extend_from_slice(&chunk);
                while frame.len() >= FRAME {
                    let prob = detector.predict_f32(&frame[..FRAME]);
                    frame.drain(..FRAME);
                    let event = segmenter.push(frame_pos, prob);
                    frame_pos += FRAME as u64;
                    match event {
                        Some(Event::Open) => {
                            seq += 1;
                            let started_at = now_ms();
                            let id = format!("mic-{started_at}-{seq}");
                            router.open(id.clone());
                            live.insert(id.clone(), Segment {
                                id: id.clone(),
                                started_at,
                                stage: Stage::Partial,
                                lang: settings.mic_language.clone(),
                                text: String::new(),
                                enriched: None,
                                settled_at: None,
                            });
                            current = Some(id);
                        }
                        Some(Event::Close { start, end, speech_ms }) => {
                            let Some(id) = current.take() else { continue };
                            router.close(&id, Instant::now() + grace);
                            if speech_ms < MIN_SPEECH_MS {
                                discard(&store, &mut live, &id);
                                continue;
                            }
                            let wav = audio::wav(&history.slice(start.saturating_sub(PADDING), end + PADDING));
                            tokio::spawn(finalize(batch.clone(), id, wav, final_tx.clone()));
                        }
                        None => {}
                    }
                }
            }
            Some(delta) = delta_rx.recv() => {
                let Some(id) = router.assign(Instant::now()) else { continue };
                if let Some(seg) = live.get_mut(&id).filter(|s| s.stage == Stage::Partial) {
                    seg.text.push_str(&delta);
                    store.upsert_segment(seg.clone());
                }
            }
            Some(done) = final_rx.recv() => {
                let Some(mut seg) = live.remove(&done.id) else { continue };
                // バッチが失敗したら Realtime で溜めたテキストを使う
                let text = done.text.unwrap_or_else(|| seg.text.clone());
                if text.trim().is_empty() {
                    seg.stage = Stage::Discarded;
                } else {
                    seg.stage = Stage::Final;
                    seg.text = text;
                    // enrich（Phase 3）が入るまでは final で確定とし、表示時間を数え始める
                    seg.settled_at = Some(now_ms());
                }
                store.upsert_segment(seg);
            }
        }
    }
    Ok(())
}

fn discard(store: &Store, live: &mut HashMap<String, Segment>, id: &str) {
    if let Some(mut seg) = live.remove(id) {
        // まだ表示していなければ流す必要もない
        if !seg.text.is_empty() {
            seg.stage = Stage::Discarded;
            store.upsert_segment(seg);
        }
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
    }
}

struct BatchContext {
    http: reqwest::Client,
    api_key: String,
    model: String,
    language: LangCode,
    context_bias: Vec<String>,
}

struct Finalized {
    id: String,
    /// None ならバッチが失敗した
    text: Option<String>,
}

async fn finalize(ctx: Arc<BatchContext>, id: String, wav: Vec<u8>, done: mpsc::Sender<Finalized>) {
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
    let _ = done.send(Finalized { id, text }).await;
}

/// 用語ヒント: 話す言語での名前と略称（DESIGN.md 8.4）
pub fn context_bias(glossary: &Glossary, lang: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for entity in glossary.entities.values() {
        let names = entity.names.get(lang).into_iter();
        let aliases = entity.aliases.get(lang).into_iter().flatten();
        for term in names.chain(aliases) {
            if !terms.contains(term) {
                terms.push(term.clone());
            }
        }
    }
    terms
}

#[derive(Debug, PartialEq)]
enum Event {
    Open,
    /// 位置は入力の先頭からのサンプル数
    Close {
        start: u64,
        end: u64,
        speech_ms: u32,
    },
}

/// VAD の確率から発話の区切りを決める（DESIGN.md 3.1）
struct Segmenter {
    min_silence_frames: u32,
    max_frames: u32,
    open: Option<Open>,
    /// 発話していないときに続いている音声フレームの数
    onset: u32,
}

struct Open {
    start: u64,
    last_speech_end: u64,
    speech_frames: u32,
    silence_frames: u32,
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
        Some(Event::Close {
            start: open.start,
            end: if hard {
                frame_end
            } else {
                open.last_speech_end
            },
            speech_ms: open.speech_frames * FRAME_MS,
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
            Event::Close {
                start: speech_start,
                end: speech_end,
                speech_ms: 800
            }
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
    }

    #[test]
    fn endless_speech_is_cut_hard() {
        let mut s = segmenter();
        let events = run(&mut s, frames(13_000, 0.9));
        assert!(matches!(events[1].1, Event::Close { .. }));
        assert_eq!(events[1].0 as u32 + 1, (8000 + 4000) / FRAME_MS);
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
    fn context_bias_collects_names_and_aliases_once() {
        let glossary: Glossary = serde_json::from_value(serde_json::json!({
            "clientVersion": 1,
            "languages": ["ja"],
            "entities": {
                "hero:13": { "kind": "hero", "className": "h", "names": { "ja": "ヘイズ", "en": "Haze" }, "aliases": { "ja": ["ヘイズ"] } },
                "item:1": { "kind": "item", "className": "i", "names": { "ja": "基礎拡張マガジン" }, "aliases": { "ja": ["拡張マガジン"] } }
            }
        }))
        .unwrap();
        assert_eq!(
            context_bias(&glossary, "ja"),
            ["ヘイズ", "基礎拡張マガジン", "拡張マガジン"]
        );
    }
}
