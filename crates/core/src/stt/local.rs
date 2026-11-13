//! PC の中で動く文字起こし（speech.cpp の Haas-ja、whisper.cpp の Whisper）で、途中の字幕を作る（DESIGN.md 9.8、9.10）。
//!
//! どちらも録音を最後まで聞いてから書くモデルなので、話している発話を、始まりから今までまとめて何度も
//! 認識し直し、そのたびに発話の文字を置き換える（`Delta::whole`）。1 回の認識は、Haas-ja が 0.02 秒、
//! Whisper が 0.2〜0.4 秒ほど（Whisper は、話している間の半分ほど GPU を使う）。
//! speech.cpp の Realtime の口（`/v1/realtime`）も試したが、2 回続けて同じだった先頭しか返さないので、
//! 言い出しが揺れる発話では何秒も文字が出なかった（実測）

use tokio::time::Instant;

use super::*;
use crate::audio;

/// 認識し直す間隔。前の認識が終わっていなければ、終わってから次を始める
const INTERVAL: Duration = Duration::from_millis(300);
/// これより短い音声は認識しない（空の文字が返るだけ）
const MIN_SAMPLES: usize = SAMPLE_RATE as usize * 3 / 10;

/// 1 セッション分。入力が尽きたら Ok、認識に失敗したら Err で返る（`super::realtime` と同じ決まり）
pub(super) async fn live(
    recognizer: &Recognizer<'_>,
    speaking: &mut Speaking,
    input: &mut mpsc::UnboundedReceiver<Input>,
    deltas: &mpsc::Sender<Delta>,
) -> anyhow::Result<()> {
    // 今の発話の、始まりからの音声と、認識に出した長さ
    let mut heard: Vec<f32> = Vec::new();
    let mut asked = 0;
    let mut next = Instant::now();
    loop {
        // 溜まった入力を先に片づける（認識している間に届いた音声を、まとめて足す）
        let waiting =
            speaking.current.is_some() && heard.len() >= MIN_SAMPLES && heard.len() > asked;
        let received = tokio::select! {
            received = input.recv() => received,
            () = tokio::time::sleep_until(next), if waiting => {
                let (segment, samples) = (speaking.current.clone().expect("speaking"), heard.len());
                let text = transcribe(recognizer, &heard).await?;
                asked = samples;
                next = Instant::now() + INTERVAL;
                if !text.is_empty() {
                    let _ = deltas.send(Delta { segment, text, whole: true }).await;
                }
                continue;
            }
        };
        match received {
            Some(Input::Open(id)) => {
                speaking.current = Some(id);
                heard = speaking.preroll.drain(..).collect();
                asked = 0;
            }
            Some(Input::Audio(chunk)) => {
                if speaking.current.is_some() {
                    heard.extend_from_slice(&chunk);
                } else {
                    speaking.remember(&chunk);
                }
            }
            Some(Input::Close) => {
                speaking.current = None;
                heard.clear();
            }
            None => return Ok(()),
        }
    }
}

/// 途中の字幕を書かせる、PC の中の文字起こし
pub(super) enum Recognizer<'a> {
    Speech(&'a Speech),
    Whisper(&'a Whisper, &'a Hints),
}

async fn transcribe(recognizer: &Recognizer<'_>, samples: &[f32]) -> anyhow::Result<String> {
    let file = reqwest::multipart::Part::bytes(audio::wav(samples))
        .file_name("utterance.wav")
        .mime_str("audio/wav")?;
    let request = match recognizer {
        Recognizer::Speech(speech) => {
            let form = reqwest::multipart::Form::new().part("file", file);
            speech.transcriptions().multipart(form)
        }
        Recognizer::Whisper(whisper, hints) => whisper
            .transcriptions()
            .multipart(whisper_form(hints).part("file", file)),
    };
    let res = request.send().await?;
    let status = res.status();
    if !status.is_success() {
        bail!("{status}: {}", res.text().await.unwrap_or_default());
    }
    text(&res.json().await?)
}
