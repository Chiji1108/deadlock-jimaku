//! 音声を流し続け、時刻の付いた文を受け取る途中の字幕（今は Qwen の Streaming だけ。開発者のサーバーの `/v2/live` も
//! 同じ形の文を送るので、アプリから頼むようにするときは、ここにつなぐ）。文の区切りは相手が決めるので、文字に付いてくる時刻（流した音声の先頭からのミリ秒）で、アプリの発話に割り当てる

use std::collections::BTreeMap;

use tokio::time::Instant;

use super::*;

/// 発話を閉じたあとに流す無音。相手が文を終わらせ、次の発話とつなげない
const SILENCE_AFTER: usize = SAMPLE_RATE as usize * 6 / 10;
/// 音声を 23 秒送らないと切られるので、黙っている間は無音を少しだけ流す（マイクの音は送らない）
const KEEP_ALIVE: Duration = Duration::from_secs(10);
const KEEP_ALIVE_SAMPLES: usize = SAMPLE_RATE as usize / 10;
/// 文を割り当てるために覚えておく発話の数
const MAX_SPANS: usize = 16;
/// 文の始まりが発話を開いた位置より少し前でも、その発話のものとみなす（時刻は 1 文字ごとに付き、少しずれる）
const EARLY: usize = SAMPLE_RATE as usize * 2 / 10;

/// 相手の文。書きかけのうちから何度も届き、届くたびに文字が置き換わる
#[derive(Debug, Default)]
pub(super) struct Sentence {
    pub id: u64,
    /// 文の始まりの時刻
    pub begin: Option<u64>,
    pub text: String,
    /// 文字ごとの時刻（ないこともある）
    pub words: Vec<Word>,
}

#[derive(Debug, Default)]
pub(super) struct Word {
    pub at: Option<u64>,
    pub text: String,
}

/// 1 つの接続の中の、流した音声と文の対応
#[derive(Default)]
pub(super) struct Session {
    /// 流したサンプル数（無音も含む）
    sent: usize,
    /// 発話を開いたときの `sent` と、その発話。古い順
    spans: VecDeque<(usize, String)>,
    /// 文の番号 → 発話ごとに分けた、今の文字（発話の順）
    sentences: BTreeMap<u64, Vec<(String, String)>>,
}

impl Session {
    fn open(&mut self, segment: String) {
        self.spans.push_back((self.sent, segment));
        if self.spans.len() > MAX_SPANS {
            self.spans.pop_front();
            let spans = &self.spans;
            self.sentences.retain(|_, parts| {
                let known =
                    |(segment, _): &(String, String)| spans.iter().any(|(_, s)| s == segment);
                parts.iter().any(known)
            });
        }
    }

    /// その時刻（ミリ秒）に開いていた発話。どの発話より前なら、いちばん古い発話
    fn segment_at(&self, ms: u64) -> Option<&String> {
        let at = ms as usize * SAMPLE_RATE as usize / 1000;
        let opened = self.spans.iter().rev();
        let mut opened = opened.skip_while(|(start, _)| *start > at + EARLY);
        let (_, segment) = opened.next().or_else(|| self.spans.front())?;
        Some(segment)
    }

    /// 届いた文を発話に割り当て、変わった発話ごとに、今の文字（文をつなげたもの）を返す。
    /// 相手の文はアプリの発話をまたぐことがある（発話の終わりで文が終わるとは限らない）ので、文字ごとの時刻で分ける
    fn on_sentence(&mut self, sentence: &Sentence) -> Vec<Delta> {
        let mut parts: Vec<(String, String)> = Vec::new();
        let mut add = |segment: Option<&String>, text: &str| {
            let Some(segment) = segment else { return };
            match parts.last_mut() {
                Some((last, said)) if last == segment => *said += text,
                _ => parts.push((segment.clone(), text.to_owned())),
            }
        };
        if sentence.words.is_empty() {
            // 文字ごとの時刻がなければ、文の始まりの時刻で決める
            let segment = sentence.begin.and_then(|ms| self.segment_at(ms));
            add(segment, &sentence.text);
        }
        for word in &sentence.words {
            add(word.at.and_then(|ms| self.segment_at(ms)), &word.text);
        }
        parts.retain(|(_, text)| !text.is_empty());
        let before = self.sentences.insert(sentence.id, parts.clone());
        // 前はあったのに消えた発話（文字が直された）も、置き換えを知らせる
        let mut changed: Vec<String> = Vec::new();
        for (segment, _) in parts.iter().chain(&before.unwrap_or_default()) {
            if !changed.contains(segment) {
                changed.push(segment.clone());
            }
        }
        let deltas = changed.into_iter().map(|segment| {
            let said = self.sentences.values().flatten();
            let text = said
                .filter(|(s, _)| *s == segment)
                .map(|(_, text)| text.as_str())
                .collect();
            Delta {
                segment,
                text,
                whole: true,
            }
        });
        deltas.collect()
    }
}

/// つないだあとの 1 セッション分。入力が尽きたら `finish` を送って Ok、切れたら Err で返る（`super::realtime` と同じ決まり）。
/// `read` は、届いた知らせから文を読む（文でなければ None。続けられない知らせなら Err）
pub(super) async fn stream(
    (mut sink, mut stream): (Sink, Stream),
    finish: String,
    mut read: impl FnMut(&Value) -> anyhow::Result<Option<Sentence>>,
    speaking: &mut Speaking,
    input: &mut mpsc::UnboundedReceiver<Input>,
    deltas: &mpsc::Sender<Delta>,
) -> anyhow::Result<()> {
    tracing::info!("realtime session started");

    // 接続している間に溜まった入力は送らない（古い言葉が今の発話に混ざる）。今の発話の続きから送る
    while let Ok(next) = input.try_recv() {
        speaking.skip(next);
    }
    let mut session = Session::default();
    let mut last_sent = Instant::now();
    macro_rules! send {
        ($samples:expr) => {{
            let samples: &[f32] = $samples;
            let pcm: Vec<u8> = samples
                .iter()
                .flat_map(|s| to_i16(*s).to_le_bytes())
                .collect();
            session.sent += samples.len();
            last_sent = Instant::now();
            sink.send(Message::binary(pcm)).await?;
        }};
    }
    if let Some(id) = speaking.current.clone() {
        session.open(id);
        let preroll: Vec<f32> = speaking.preroll.drain(..).collect();
        send!(&preroll);
    }
    loop {
        tokio::select! {
            next = input.recv() => match next {
                Some(Input::Open(id)) => {
                    speaking.current = Some(id.clone());
                    session.open(id);
                    let preroll: Vec<f32> = speaking.preroll.drain(..).collect();
                    send!(&preroll);
                }
                Some(Input::Audio(chunk)) => {
                    if speaking.current.is_some() {
                        send!(&chunk);
                    } else {
                        speaking.remember(&chunk);
                    }
                }
                Some(Input::Close) => {
                    if speaking.current.take().is_some() {
                        send!(&[0.0; SILENCE_AFTER]);
                    }
                }
                None => {
                    let _ = sink.send(Message::text(finish)).await;
                    return Ok(());
                }
            },
            () = tokio::time::sleep_until(last_sent + KEEP_ALIVE) => send!(&[0.0; KEEP_ALIVE_SAMPLES]),
            msg = stream.next() => {
                let event = match msg {
                    Some(Ok(Message::Text(text))) => serde_json::from_str::<Value>(&text)?,
                    Some(Ok(Message::Close(frame))) => bail!("realtime closed: {frame:?}"),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(e.into()),
                    None => bail!("realtime stream ended"),
                };
                if let Some(sentence) = read(&event)? {
                    for delta in session.on_sentence(&sentence) {
                        let _ = deltas.send(delta).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 文字ごとの時刻（ミリ秒）と文字
    fn sentence(id: u64, words: &[(u64, &str)]) -> Sentence {
        let words = words.iter().map(|(ms, text)| Word {
            at: Some(*ms),
            text: (*text).to_owned(),
        });
        Sentence {
            id,
            begin: Some(0),
            text: String::new(),
            words: words.collect(),
        }
    }

    fn said(session: &mut Session, sentence: Sentence) -> Vec<(String, String)> {
        let deltas = session.on_sentence(&sentence).into_iter();
        deltas.map(|d| (d.segment, d.text)).collect()
    }

    fn of(segment: &str, text: &str) -> (String, String) {
        (segment.to_owned(), text.to_owned())
    }

    #[test]
    fn words_go_to_the_utterance_that_was_open_when_they_were_said() {
        let mut session = Session::default();
        session.open("a".into());
        session.sent += 3 * SAMPLE_RATE as usize;
        session.open("b".into());
        // 書きかけの文は、届くたびに置き換わる
        assert_eq!(
            said(&mut session, sentence(1, &[(400, "さあ")])),
            [of("a", "さあ")]
        );
        let first = sentence(1, &[(400, "さあ"), (900, "始まりました。")]);
        assert_eq!(said(&mut session, first), [of("a", "さあ始まりました。")]);
        // 同じ発話の 2 つ目の文は後ろにつなげる。発話をまたいだ文は、文字の時刻で分ける
        // （次の発話を開いた位置より少し前の文字も、次の発話のもの）
        let second = sentence(2, &[(1800, "実況です。"), (2900, "ヘイズ")]);
        assert_eq!(
            said(&mut session, second),
            [of("a", "さあ始まりました。実況です。"), of("b", "ヘイズ")]
        );
        // 文字が直されて発話から消えたら、その発話も置き換える
        let fixed = sentence(2, &[(1800, "実況です。")]);
        assert_eq!(
            said(&mut session, fixed),
            [of("a", "さあ始まりました。実況です。"), of("b", "")]
        );
        // 文字のない知らせ（文の始まり）は何も出さない
        assert!(said(&mut session, sentence(3, &[])).is_empty());
    }

    #[test]
    fn a_sentence_without_word_times_goes_by_its_beginning() {
        let mut session = Session::default();
        session.open("a".into());
        let whole = Sentence {
            id: 1,
            begin: Some(500),
            text: "さあ".into(),
            words: Vec::new(),
        };
        assert_eq!(said(&mut session, whole), [of("a", "さあ")]);
    }
}
