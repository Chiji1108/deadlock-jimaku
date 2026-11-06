//! ローカルAI のモード（DESIGN.md 9.6）。途中の字幕と途中の翻訳、用語の判定を、PC の中の補助プログラムで作る。
//!
//! 補助プログラム（`crates/local-ai`）は実行環境が大きいので、アプリに入れず、使う人だけ初回に
//! ダウンロードする。別のプロセスにすると、使わない人のメモリも増えない

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::settings::{Settings, SpeechEngine};
use crate::store::Store;
use crate::subtitles::Done;
use crate::types::{Entity, Glossary, LiveState, LiveStatus, LiveText, Token, Translations};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "https://github.com/Chiji1108/deadlock-jimaku/releases/download";
/// 開発中は、手元でビルドした補助プログラムを使う（`mise run local`）
const DEV_DIR: &str = "JIMAKU_LOCAL_DIR";
/// 補助プログラムのアーカイブの SHA-256。CI がビルドのときに埋め込む（手元のビルドにはないので、
/// `JIMAKU_LOCAL_DIR` を使う）
const ARCHIVE_SHA256: Option<&str> = if cfg!(windows) {
    option_env!("JIMAKU_LOCAL_SHA256_WINDOWS_X86_64")
} else if cfg!(target_os = "linux") {
    option_env!("JIMAKU_LOCAL_SHA256_LINUX_X86_64")
} else if cfg!(target_arch = "aarch64") {
    option_env!("JIMAKU_LOCAL_SHA256_MACOS_AARCH64")
} else {
    option_env!("JIMAKU_LOCAL_SHA256_MACOS_X86_64")
};
/// 遅い回線でも、大きなファイルを受け取れるようにする
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// 翻訳できる言語。日本語から英語を経由する
const TARGETS: [&str; 4] = ["en", "ru", "zh-Hans", "zh-Hant"];
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const AUDIO: u8 = 1;
const CUT: u8 = 2;
const MARK: u8 = 3;
const TERMS: u8 = 4;

/// 字幕の処理が持つ。落とすと補助プログラムの処理も止まる
pub struct Task {
    store: Arc<Store>,
    handle: JoinHandle<()>,
}

impl Task {
    pub(crate) fn spawn(
        store: Arc<Store>,
        settings: Settings,
        done: mpsc::Sender<Done>,
        input: mpsc::UnboundedReceiver<Input>,
    ) -> Self {
        let handle = tokio::spawn(run(store.clone(), settings, done, input));
        Self { store, handle }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.handle.abort();
        self.store.set_live_status(None);
    }
}

pub enum Input {
    Audio(Vec<f32>),
    /// 発話が閉じた
    Cut,
    /// OpenAI の確定の文に、読みと用語を付ける（`Done::Marked` で返る）
    Mark {
        id: String,
        text: String,
    },
    /// 用語の表を入れ替える
    Terms(Vec<Term>),
}

/// 補助プログラムからの 1 行。形が合わなければ `session` が止まって知らせる（空の字幕を出さない）
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Message {
    Progress { loaded: u64, total: u64 },
    Ready,
    Text(Final),
    Final(Final),
    Marked { id: String, tokens: Vec<Token> },
}

/// 補助プログラムが認識した文。ローカルAIのモードの確定の字幕では、文字が空なら、その発話を捨てる
#[derive(Debug, Default, Deserialize)]
pub struct Final {
    pub text: String,
    pub tokens: Vec<Token>,
    pub translations: Translations,
}

/// 辞書の 1 つの名前。補助プログラムが字幕の中から探し、翻訳の前に英語の名前へ置き換える
#[derive(Debug, PartialEq, Serialize)]
pub struct Term {
    #[serde(rename = "ref")]
    reference: String,
    text: String,
    /// かなの読み。辞書に読みがなければ名前のまま（漢字を含むなら、読みでは探さない）
    reading: String,
    en: String,
}

/// 日本語と英語の公式名がある、すべての用語
pub fn terms(glossary: &Glossary) -> Vec<Term> {
    let mut terms: Vec<Term> = glossary
        .entities
        .iter()
        .filter_map(|(reference, e)| {
            let (name, en) = (e.names.get("ja")?, e.names.get("en")?);
            Some(Term {
                reference: reference.clone(),
                text: name.clone(),
                reading: reading(e).unwrap_or_else(|| name.clone()),
                en: en.clone(),
            })
        })
        .collect();
    // スキルと同じ名前のアイテムがある（ライフドレイン）。先の行が勝つので、説明を出せるアイテムを先にする
    terms.sort_by_key(|t| !t.reference.starts_with("item:"));
    terms
}

fn reading(entity: &Entity) -> Option<String> {
    let ruby = entity.ruby.as_ref()?.iter();
    Some(
        ruby.map(|s| s.ruby.as_ref().unwrap_or(&s.text).as_str())
            .collect(),
    )
}

async fn run(
    store: Arc<Store>,
    settings: Settings,
    done: mpsc::Sender<Done>,
    mut input: mpsc::UnboundedReceiver<Input>,
) {
    show(&store, LiveState::Preparing, None);
    let finals = settings.speech_engine == SpeechEngine::Local;
    // 確定の字幕を返していない発話の数。ローカルAIのモードの字幕の処理は、区切りごとの返事を待っている
    let mut waiting = 0;
    let result = session(&store, &settings, finals, &done, &mut input, &mut waiting);
    if let Err(e) = result.await {
        tracing::warn!("local ai: {e:#}");
        show(
            &store,
            LiveState::Failed,
            Some(format!("ローカルAIを使えません: {e}")),
        );
    }
    // 止まったあとも区切りには返事をする（字幕の処理が待ち続けないように）
    if !finals {
        return;
    }
    for _ in 0..waiting {
        let _ = done.send(Done::Local(Final::default())).await;
    }
    while let Some(next) = input.recv().await {
        if matches!(next, Input::Cut) {
            let _ = done.send(Done::Local(Final::default())).await;
        }
    }
}

fn show(store: &Store, state: LiveState, detail: Option<String>) {
    store.set_live_status(Some(LiveStatus { state, detail }));
}

async fn session(
    store: &Arc<Store>,
    settings: &Settings,
    finals: bool,
    done: &mpsc::Sender<Done>,
    input: &mut mpsc::UnboundedReceiver<Input>,
    waiting: &mut usize,
) -> anyhow::Result<()> {
    if settings.mic_language != "ja" {
        bail!("話す言語が日本語のときだけ使えます");
    }
    let root = store.data_dir().join("local-ai");
    tokio::fs::create_dir_all(&root).await?;
    let dir = install(store, &root).await?;
    let targets: Vec<&str> = TARGETS
        .into_iter()
        .filter(|t| settings.translate_to.iter().any(|l| l == t))
        .collect();
    let mut command = Command::new(dir.join(binary()));
    command
        .arg(root.join("models"))
        .arg(targets.join(","))
        .arg(if finals { "final" } else { "partial" })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // コンソールの窓を出さない（全画面のゲームからフォーカスを奪う）
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn().context("start")?;
    let stdin = child.stdin.take().context("stdin")?;
    let mut stderr = child.stderr.take().context("stderr")?;
    // 書き込みで止まっても、出力を読み続ける
    let (writer, rx) = mpsc::unbounded_channel();
    tokio::spawn(write_input(stdin, rx));
    let mut lines = BufReader::new(child.stdout.take().context("stdout")?).lines();
    // 準備が終わるまでの音声は渡さない（古い発話の字幕が今の発話に載る）
    let mut ready = false;
    // 辞書はあとから届いたり、読みが増えて作り直されたりする。準備ができたら今の辞書から渡す
    let mut glossary = store.watch_glossary();
    glossary.mark_changed();
    loop {
        tokio::select! {
            next = input.recv() => {
                let Some(next) = next else { return Ok(()) };
                let cut = matches!(next, Input::Cut);
                if ready {
                    *waiting += usize::from(cut && finals);
                    let _ = writer.send(next);
                } else if cut && finals {
                    let _ = done.send(Done::Local(Final::default())).await;
                }
            }
            Ok(()) = glossary.changed(), if ready && settings.deadlock_mode => {
                let terms = glossary
                    .borrow_and_update()
                    .as_ref()
                    .map(|l| terms(&l.glossary))
                    .unwrap_or_default();
                let _ = writer.send(Input::Terms(terms));
            }
            line = lines.next_line() => {
                let Some(line) = line? else {
                    let mut reason = String::new();
                    stderr.read_to_string(&mut reason).await?;
                    bail!("{}", reason.trim());
                };
                let message = serde_json::from_str(&line).context("補助プログラムの版が合いません")?;
                match message {
                    Message::Progress { loaded, total } => show(
                        store,
                        LiveState::Preparing,
                        Some(format!(
                            "モデルをダウンロードしています（{}%）",
                            (loaded * 100 / total.max(1)).min(100)
                        )),
                    ),
                    Message::Ready => {
                        ready = true;
                        show(store, LiveState::Listening, None);
                    }
                    Message::Text(Final { text, tokens, translations }) => {
                        store.send_live(LiveText { text, tokens, translations });
                    }
                    Message::Final(result) if finals => {
                        *waiting -= 1;
                        let _ = done.send(Done::Local(result)).await;
                    }
                    Message::Final(_) => {}
                    Message::Marked { id, tokens } => {
                        let _ = done.send(Done::Marked { id, tokens }).await;
                    }
                }
            }
        }
    }
}

async fn write_input(mut stdin: ChildStdin, mut rx: mpsc::UnboundedReceiver<Input>) {
    while let Some(input) = rx.recv().await {
        if send(&mut stdin, input).await.is_err() {
            return;
        }
    }
}

async fn send(stdin: &mut ChildStdin, input: Input) -> std::io::Result<()> {
    match input {
        Input::Audio(samples) => {
            let mut frame = Vec::with_capacity(5 + samples.len() * 4);
            frame.push(AUDIO);
            frame.extend_from_slice(&((samples.len() * 4) as u32).to_le_bytes());
            frame.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
            stdin.write_all(&frame).await
        }
        Input::Cut => stdin.write_all(&[CUT]).await,
        Input::Mark { id, text } => {
            let body = format!("{id}\n{text}");
            stdin.write_all(&frame(MARK, body.as_bytes())).await
        }
        Input::Terms(terms) => {
            let body = serde_json::to_vec(&terms)?;
            stdin.write_all(&frame(TERMS, &body)).await
        }
    }
}

fn frame(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5 + body.len());
    frame.push(kind);
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(body);
    frame
}

fn binary() -> &'static str {
    if cfg!(windows) {
        "jimaku-local.exe"
    } else {
        "jimaku-local"
    }
}

/// 補助プログラムのあるフォルダ。なければリリースからダウンロードする
async fn install(store: &Store, root: &Path) -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os(DEV_DIR) {
        return Ok(dir.into());
    }
    let dir = root.join(VERSION);
    if dir.join(binary()).exists() {
        return Ok(dir);
    }
    show(
        store,
        LiveState::Preparing,
        Some("ローカルAI をダウンロードしています".into()),
    );
    let archive = format!(
        "jimaku-local-{}-{}.tar.gz",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let expected = ARCHIVE_SHA256.context("no checksum in this build")?;
    let url = format!("{RELEASES}/v{VERSION}/{archive}");
    let bytes = crate::http()
        .get(&url)
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    if !checksum_matches(&bytes, expected) {
        bail!("{archive}: the checksum does not match");
    }
    let extract_to = dir.clone();
    tokio::task::spawn_blocking(move || extract(&bytes, &extract_to)).await??;
    Ok(dir)
}

fn checksum_matches(bytes: &[u8], expected: &str) -> bool {
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    digest == expected.trim()
}

/// 途中で止まっても半端なフォルダを残さないよう、できてから置く
fn extract(bytes: &[u8], dir: &Path) -> anyhow::Result<()> {
    let partial = dir.with_extension(format!("partial-{}", crate::now_ms()));
    std::fs::create_dir_all(&partial)?;
    tar::Archive::new(GzDecoder::new(bytes)).unpack(&partial)?;
    let renamed = std::fs::rename(&partial, dir);
    // 同時に別の展開が終わっていたら、それを使う
    if renamed.is_err() && dir.exists() {
        std::fs::remove_dir_all(partial)?;
        return Ok(());
    }
    Ok(renamed?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EntityKind, RubySegment};

    fn magazine(ruby: Option<Vec<RubySegment>>) -> Glossary {
        let entity = Entity {
            kind: EntityKind::Item,
            class_name: "upgrade_clip_size".into(),
            names: [("ja", "基礎拡張マガジン"), ("en", "Extended Magazine")]
                .map(|(lang, name)| (lang.to_owned(), name.to_owned()))
                .into(),
            ruby,
            icon: None,
            slot: None,
            hero: None,
            ability_slot: None,
        };
        Glossary {
            client_version: 1,
            languages: Vec::new(),
            entities: [("item:1548066885".to_owned(), entity)].into(),
        }
    }

    #[test]
    fn a_term_has_the_reading_of_its_name() {
        let ruby = vec![
            RubySegment {
                text: "基礎拡張".into(),
                ruby: Some("きそかくちょう".into()),
            },
            RubySegment {
                text: "マガジン".into(),
                ruby: None,
            },
        ];
        let terms = terms(&magazine(Some(ruby)));
        assert_eq!(terms[0].reading, "きそかくちょうマガジン");
        assert_eq!(terms[0].en, "Extended Magazine");
    }

    #[test]
    fn a_name_that_is_an_ordinary_word_is_a_term() {
        let mut glossary = magazine(None);
        let entity = glossary.entities.values_mut().next().unwrap();
        entity.names.insert("ja".into(), "ポケット".into());
        assert_eq!(terms(&glossary)[0].text, "ポケット");
    }

    #[test]
    fn items_come_before_abilities_of_the_same_name() {
        let mut glossary = magazine(None);
        let mut ability = glossary.entities.values().next().unwrap().clone();
        ability.kind = EntityKind::Ability;
        glossary.entities.insert("ability:1".into(), ability);
        assert_eq!(terms(&glossary)[0].reference, "item:1548066885");
    }

    #[test]
    fn a_name_without_a_reading_is_listed_as_written() {
        assert_eq!(terms(&magazine(None))[0].reading, "基礎拡張マガジン");
    }

    #[test]
    fn helper_lines_become_app_tokens() {
        let line = serde_json::json!({
            "type": "final",
            "text": "基礎拡張マガジンを",
            "tokens": [{ "t": "entity", "ref": "item:1", "spoken": "基礎拡張マガジン" }, { "t": "text", "text": "を" }],
            "translations": { "en": [{ "t": "entity", "ref": "item:1" }] },
        });
        let Message::Final(result) = serde_json::from_value(line).unwrap() else {
            panic!("not a final");
        };
        assert_eq!(
            result.tokens,
            [
                Token::Entity {
                    reference: "item:1".into(),
                    spoken: Some("基礎拡張マガジン".into()),
                    ruby: None,
                },
                Token::Text {
                    text: "を".into(),
                    ruby: None,
                },
            ]
        );
        assert_eq!(result.translations.keys().collect::<Vec<_>>(), ["en"]);
    }

    #[test]
    fn a_line_of_another_shape_is_an_error() {
        for line in [
            r#"{ "type": "text", "text": "あ", "tokens": "あ", "translations": {} }"#,
            r#"{ "type": "final", "text": "あ" }"#,
            r#"{ "type": "marked", "id": "1", "tokens": [{ "t": "word" }] }"#,
            r#"{ "type": "other" }"#,
        ] {
            assert!(serde_json::from_str::<Message>(line).is_err(), "{line}");
        }
    }

    #[test]
    fn checksum_detects_a_changed_file() {
        let sha256 = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(checksum_matches(b"hello", sha256));
        assert!(!checksum_matches(b"hello!", sha256));
    }

    #[test]
    fn extracts_the_archive_into_place() {
        let dir = tempfile::tempdir().unwrap();
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(2);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, "jimaku-local", &b"hi"[..])
            .unwrap();
        let bytes = archive.into_inner().unwrap().finish().unwrap();
        let target = dir.path().join("0.3.0");
        extract(&bytes, &target).unwrap();
        assert_eq!(std::fs::read(target.join("jimaku-local")).unwrap(), b"hi");
    }
}
