//! 音声入力（DESIGN.md 4 章）。どの入力も 16kHz mono の f32 に揃えて流す。

use std::collections::VecDeque;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use audioadapter_buffers::direct::InterleavedSlice;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::{Async, FixedAsync, Indexing, PolynomialDegree, Resampler as _};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use ts_rs::TS;

pub const SAMPLE_RATE: u32 = 16_000;

/// マイクを開けない理由。そのまま操作パネルに送り、操作パネルが文にする（`TranscriptionError`）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "problem", rename_all = "camelCase")]
#[ts(export)]
pub enum MicError {
    /// 入力デバイスの一覧を取れない（`detail` は OS のエラー）
    ListFailed {
        detail: String,
    },
    NoDevices,
    NotFound {
        name: String,
    },
    NoDefault,
}

/// ログ用（画面には出さない）
impl std::fmt::Display for MicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for MicError {}

pub enum Input {
    /// None なら OS の既定の入力デバイス
    Mic(Option<String>),
    /// 開発・検証用。実時間と同じ速さで流す
    File(PathBuf),
}

/// 入力を開き、16kHz mono のサンプルを流すチャンネルを返す。開いた時点から流れ始める
pub async fn open(input: Input) -> anyhow::Result<mpsc::Receiver<Vec<f32>>> {
    let (tx, rx) = mpsc::channel(256);
    // デバイスの初期化やファイルの読み込みは同期処理なので、非同期のワーカーを塞がない
    match input {
        Input::Mic(device) => tokio::task::spawn_blocking(move || open_mic(device, tx)).await??,
        Input::File(path) => {
            let audio = tokio::task::spawn_blocking(move || load_file(&path)).await??;
            tokio::spawn(play(audio, tx));
        }
    }
    Ok(rx)
}

/// 入力デバイスの名前（操作パネルの選択肢）
pub fn input_devices() -> anyhow::Result<Vec<String>> {
    let mut names: Vec<String> = cpal::default_host()
        .input_devices()?
        .filter_map(|d| Some(d.description().ok()?.name().to_owned()))
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

fn open_mic(device: Option<String>, tx: mpsc::Sender<Vec<f32>>) -> anyhow::Result<()> {
    // cpal の Stream は Send でないプラットフォームがあるので、専用のスレッドで持ち続ける
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("mic".into())
        .spawn(move || {
            let stream = match build_mic_stream(device.as_deref(), tx.clone()) {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    stream
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            // 受け手がいなくなったら止める
            while !tx.is_closed() {
                std::thread::sleep(Duration::from_millis(200));
            }
            drop(stream);
        })?;
    ready_rx.recv().context("mic thread exited")?
}

fn build_mic_stream(
    name: Option<&str>,
    tx: mpsc::Sender<Vec<f32>>,
) -> anyhow::Result<cpal::Stream> {
    let host = cpal::default_host();
    // 入力デバイスがないと、既定のデバイスとして使えないものが返ってくることがある（macOS）。
    // 先に一覧を見て、操作パネルで分かる理由にする
    let mut devices = host
        .input_devices()
        .map_err(|e| MicError::ListFailed {
            detail: e.to_string(),
        })?
        .peekable();
    if devices.peek().is_none() {
        return Err(MicError::NoDevices.into());
    }
    let device = match name {
        Some(name) => devices
            .find(|d| d.description().is_ok_and(|desc| desc.name() == name))
            .ok_or_else(|| MicError::NotFound {
                name: name.to_owned(),
            })?,
        None => host.default_input_device().ok_or(MicError::NoDefault)?,
    };
    let config = device.default_input_config()?;
    tracing::info!(
        "mic: {} ({} Hz, {} ch, {:?})",
        device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_default(),
        config.sample_rate(),
        config.channels(),
        config.sample_format(),
    );

    let channels = usize::from(config.channels());
    let mut resampler = Resampler::new(config.sample_rate())?;
    // コールバックは実時間のスレッドで呼ばれるので、作業用のバッファは使い回す
    let mut mono = Vec::new();
    let mut on_samples = move |mono: &[f32]| {
        let out = resampler.process(mono);
        // 処理が詰まったら古い音声より今の音声を優先して捨てる
        if !out.is_empty() && tx.try_send(out).is_err() {
            tracing::warn!("audio dropped");
        }
    };
    let on_error = |e: cpal::Error| tracing::error!("mic: {e}");

    macro_rules! build {
        ($t:ty) => {
            device.build_input_stream(
                config.clone().into(),
                move |data: &[$t], _: &_| {
                    downmix_into(data, channels, &mut mono);
                    on_samples(&mono);
                },
                on_error,
                None,
            )?
        };
    }
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::I32 => build!(i32),
        cpal::SampleFormat::U16 => build!(u16),
        other => anyhow::bail!("unsupported sample format {other:?}"),
    };
    stream.play()?;
    Ok(stream)
}

fn downmix_into<T: cpal::Sample>(data: &[T], channels: usize, out: &mut Vec<f32>)
where
    f32: cpal::FromSample<T>,
{
    out.clear();
    out.extend(
        data.chunks(channels)
            .map(|frame| frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32),
    );
}

/// WAV を読み、16kHz mono に揃える
fn load_file(path: &Path) -> anyhow::Result<Vec<f32>> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("open {}", path.display()))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = (1_i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|s| s as f32 / scale))
                .collect::<Result<_, _>>()?
        }
    };
    let mut mono = Vec::new();
    downmix_into(&samples, usize::from(spec.channels), &mut mono);
    let mut audio = Resampler::new(spec.sample_rate)?.process(&mono);
    // 最後の発話が無音で閉じるよう、末尾に 2 秒の無音を足す
    audio.extend(std::iter::repeat_n(0.0, SAMPLE_RATE as usize * 2));
    Ok(audio)
}

async fn play(audio: Vec<f32>, tx: mpsc::Sender<Vec<f32>>) {
    const CHUNK: usize = SAMPLE_RATE as usize / 50; // 20ms
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    // 受け手が詰まって遅れても、取り戻すために一気に流さない（実時間を保つ）
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    for chunk in audio.chunks(CHUNK) {
        tick.tick().await;
        if tx.send(chunk.to_vec()).await.is_err() {
            return;
        }
    }
}

/// 任意のサンプルレートの mono を 16kHz に変換する
struct Resampler {
    inner: Option<Async<f32>>,
    pending: Vec<f32>,
    buf: Vec<f32>,
}

impl Resampler {
    fn new(rate: u32) -> anyhow::Result<Self> {
        let inner = (rate != SAMPLE_RATE)
            .then(|| {
                Async::<f32>::new_poly(
                    f64::from(SAMPLE_RATE) / f64::from(rate),
                    1.1,
                    PolynomialDegree::Cubic,
                    1024,
                    1,
                    FixedAsync::Input,
                )
            })
            .transpose()?;
        let buf = vec![0.0; inner.as_ref().map_or(0, |r| r.output_frames_max())];
        Ok(Self {
            inner,
            pending: Vec::new(),
            buf,
        })
    }

    /// 変換した音声。PCM の範囲（-1〜1）に収める（大きな音や変換のはみ出しで範囲を超えると、
    /// VAD（earshot）が受け付けない。範囲を超えた分はマイクでも切れているので、切るのが本来の値）
    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let mut out = self.resample(input);
        for s in &mut out {
            // NaN（壊れたファイルなど）は無音にする（clamp は NaN を通してしまう）
            *s = if s.is_nan() { 0.0 } else { s.clamp(-1.0, 1.0) };
        }
        out
    }

    fn resample(&mut self, input: &[f32]) -> Vec<f32> {
        let Some(inner) = &mut self.inner else {
            return input.to_vec();
        };
        self.pending.extend_from_slice(input);
        let mut out = Vec::new();
        let indexing = Indexing::new();
        while self.pending.len() >= inner.input_frames_next() {
            let n = inner.input_frames_next();
            let input = InterleavedSlice::new(&self.pending[..n], 1, n).expect("sized");
            let cap = self.buf.len();
            let mut output = InterleavedSlice::new_mut(&mut self.buf, 1, cap).expect("sized");
            match inner.process_into_buffer(&input, &mut output, Some(&indexing)) {
                Ok((_, written)) => out.extend_from_slice(&self.buf[..written]),
                Err(e) => tracing::error!("resample: {e}"),
            }
            self.pending.drain(..n);
        }
        out
    }
}

/// 直近の音声を持っておき、Segment の区間を切り出す。位置は入力の先頭からのサンプル数
pub struct History {
    samples: VecDeque<f32>,
    /// samples[0] の位置
    start: u64,
    capacity: usize,
}

impl History {
    pub fn new(seconds: u32) -> Self {
        let capacity = (SAMPLE_RATE * seconds) as usize;
        Self {
            samples: VecDeque::with_capacity(capacity),
            start: 0,
            capacity,
        }
    }

    pub fn push(&mut self, chunk: &[f32]) {
        self.samples.extend(chunk);
        let overflow = self.samples.len().saturating_sub(self.capacity);
        self.samples.drain(..overflow);
        self.start += overflow as u64;
    }

    /// 入力の先頭からのサンプル数（次に push されるサンプルの位置）
    pub fn end(&self) -> u64 {
        self.start + self.samples.len() as u64
    }

    /// [from, to) を切り出す。範囲外は詰める
    pub fn slice(&self, from: u64, to: u64) -> Vec<f32> {
        let from = from.clamp(self.start, self.end());
        let to = to.clamp(from, self.end());
        self.samples
            .range((from - self.start) as usize..(to - self.start) as usize)
            .copied()
            .collect()
    }
}

/// 16kHz mono 16bit の WAV
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("in-memory");
    for s in samples {
        writer.write_sample(to_i16(*s)).expect("in-memory");
    }
    writer.finalize().expect("in-memory");
    cursor.into_inner()
}

pub fn to_i16(s: f32) -> i16 {
    (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keeps_recent_and_slices_by_position() {
        let mut h = History::new(1);
        let one_sec: Vec<f32> = (0..SAMPLE_RATE).map(|i| i as f32).collect();
        h.push(&one_sec);
        h.push(&[1.0, 2.0]);
        assert_eq!(h.end(), u64::from(SAMPLE_RATE) + 2);
        // 先頭 2 サンプルは捨てられているので、[0, 3) で残っているのは位置 2 だけ
        assert_eq!(h.slice(0, 3), vec![2.0]);
        assert_eq!(h.slice(h.end() - 2, h.end() + 10), vec![1.0, 2.0]);
    }

    #[test]
    fn loud_input_stays_within_pcm_range() {
        // 大きな音（範囲外の値）や、変換でのはみ出しがあっても -1〜1 に収まる
        let mut loud: Vec<f32> = (0..48_000)
            .map(|i| if i % 2 == 0 { 3.0 } else { -3.0 })
            .collect();
        loud[100] = f32::NAN;
        for rate in [48_000, SAMPLE_RATE] {
            let out = Resampler::new(rate).unwrap().process(&loud);
            assert!(!out.is_empty());
            assert!(out.iter().all(|s| (-1.0..=1.0).contains(s)), "{rate}");
        }
    }

    #[test]
    fn resampler_converts_rate() {
        let mut r = Resampler::new(48_000).unwrap();
        let out = r.process(&vec![0.0; 48_000]);
        // 内部の遅延とチャンク待ちがあるので、おおよそ 1/3
        assert!((15_000..=16_100).contains(&out.len()), "{}", out.len());
    }

    #[test]
    fn wav_has_header_and_samples() {
        let bytes = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 3 * 2);
    }
}
