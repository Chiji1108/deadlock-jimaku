//! 発話の区切り。アプリのマイクの音声を、VAD で発話（開く〜閉じる）に分ける（DESIGN.md 3.1）。
//!
//! 確定の文字起こしをアプリが作るときは、ここが字幕の区切りを決める（区切る無音の長さは設定の
//! `segmentation`）。途中の字幕（OpenAI の Realtime、Qwen の Streaming、Chrome）は、それぞれのやり方で
//! この区切りに合わせる（`stt/openai.rs` の `Items`、`stt/qwen.rs` の `Session`、`Store::cut_chrome`）。
//! 確定を Chrome が作るときだけは、Chrome のページが区切る（`frontend/src/chrome/speech.ts`）

use crate::settings::Segmentation;

/// earshot が受け取るフレームの長さ（16ms）
pub const FRAME: usize = 256;
pub const FRAME_MS: u32 = 16;

#[derive(Debug, PartialEq)]
pub enum Event {
    Open,
    Close(Close),
}

/// 位置は入力の先頭からのサンプル数
#[derive(Debug, PartialEq)]
pub struct Close {
    pub start: u64,
    pub end: u64,
    pub speech_ms: u32,
    /// 前の発話を強制的に切った直後から始まっている
    pub cut_start: bool,
    /// 発話の途中で強制的に切った
    pub cut_end: bool,
}

/// 音声から発話の区切りを決める（DESIGN.md 3.1）。VAD（earshot）にフレームごとの確率を出させ、
/// 無音の長さと発話の長さで開く・閉じるを決める
pub struct Segmenter {
    detector: earshot::Detector,
    /// VAD に渡すフレームに満たない端数
    frame: Vec<f32>,
    /// 次のフレームの先頭位置
    frame_pos: u64,
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

    pub fn new(cfg: &Segmentation) -> Self {
        Self {
            detector: earshot::Detector::default(),
            frame: Vec::with_capacity(FRAME * 2),
            frame_pos: 0,
            min_silence_frames: cfg.min_silence_ms / FRAME_MS,
            max_frames: cfg.max_segment_ms / FRAME_MS,
            open: None,
            onset: 0,
            last_cut: None,
        }
    }

    /// 音声を足し、その間に起きた開く・閉じるを順に返す
    pub fn feed(&mut self, chunk: &[f32]) -> Vec<Event> {
        let mut events = Vec::new();
        self.frame.extend_from_slice(chunk);
        while self.frame.len() >= FRAME {
            let prob = self.detector.predict_f32(&self.frame[..FRAME]);
            self.frame.drain(..FRAME);
            events.extend(self.push(self.frame_pos, prob));
            self.frame_pos += FRAME as u64;
        }
        events
    }

    /// VAD の確率を 1 フレーム分足す。`pos` はこのフレームの先頭位置
    pub(crate) fn push(&mut self, pos: u64, prob: f32) -> Option<Event> {
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
    pub fn finish(&mut self) -> Option<Close> {
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
}
