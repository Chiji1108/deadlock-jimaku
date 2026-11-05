//! モデルを初回にダウンロードする。中身は SHA-256 で確かめる

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};

const SPEECH_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2";
const SPEECH_SHA256: &str = "7d1efa2138a65b0b488df37f8b89e3d91a60676e416f515b952358d83dfd347e";
const SPEECH_FILES: [&str; 2] = ["model.int8.onnx", "tokens.txt"];

/// Firefox の翻訳モデル（Bergamot、日本語から英語）
const TRANSLATION_BASE: &str =
    "https://firefox-settings-attachments.cdn.mozilla.net/main-workspace/translations-models-v2";
const TRANSLATION_FILES: [(&str, &str, &str); 3] = [
    (
        "model.bin",
        "53a52e41-5abb-4705-9ba8-8dbc07614bdc.zst",
        "c0c9f2e48979f022e16271049d96dbef9aa28f8fddc2d1db8fa0c399dec903f7",
    ),
    (
        "vocab.spm",
        "2566c4c6-4b9f-4d94-8898-38dbda231813.zst",
        "f41f32e36f879643a47fedbd0616fa12f416da62e23af8620ee4fc7705217acc",
    ),
    (
        "lex.bin",
        "3fe6eec6-c9fa-4262-85d1-a773ece70b6c.zst",
        "e6715f3e8a9d2ff08004b8185305de73906d7c31352f99a7f0424b4bf96a9422",
    ),
];
/// 進み具合の表示用（全部で約 210MB）
const TOTAL_BYTES: u64 = 210_000_000;

pub struct Paths {
    pub speech_model: PathBuf,
    pub tokens: PathBuf,
    pub translation: PathBuf,
    pub vocabulary: PathBuf,
    pub shortlist: PathBuf,
}

pub fn ensure(dir: &Path, mut progress: impl FnMut(u64, u64)) -> anyhow::Result<Paths> {
    fs::create_dir_all(dir)?;
    let paths = Paths {
        speech_model: dir.join(SPEECH_FILES[0]),
        tokens: dir.join(SPEECH_FILES[1]),
        translation: dir.join(TRANSLATION_FILES[0].0),
        vocabulary: dir.join(TRANSLATION_FILES[1].0),
        shortlist: dir.join(TRANSLATION_FILES[2].0),
    };
    let mut loaded = 0;
    if !(paths.speech_model.exists() && paths.tokens.exists()) {
        let archive = download(SPEECH_URL, SPEECH_SHA256, &mut loaded, &mut progress)?;
        extract_speech(&archive, dir)?;
    }
    for (name, file, sha256) in TRANSLATION_FILES {
        let target = dir.join(name);
        if target.exists() {
            continue;
        }
        let compressed = download(
            &format!("{TRANSLATION_BASE}/{file}"),
            sha256,
            &mut loaded,
            &mut progress,
        )?;
        let mut decoder = ruzstd::decoding::StreamingDecoder::new(compressed.as_slice())?;
        let mut bytes = Vec::new();
        decoder.read_to_end(&mut bytes)?;
        write_atomic(&target, &bytes)?;
    }
    Ok(paths)
}

fn download(
    url: &str,
    sha256: &str,
    loaded: &mut u64,
    progress: &mut impl FnMut(u64, u64),
) -> anyhow::Result<Vec<u8>> {
    let mut response = ureq::get(url).call().with_context(|| url.to_owned())?;
    let mut reader = response.body_mut().with_config().limit(u64::MAX).reader();
    let mut bytes = Vec::new();
    let mut chunk = vec![0; 1 << 20];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        *loaded += read as u64;
        progress(*loaded, TOTAL_BYTES);
    }
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if digest != sha256 {
        bail!("{url}: the checksum does not match");
    }
    Ok(bytes)
}

fn extract_speech(archive: &[u8], dir: &Path) -> anyhow::Result<()> {
    let decoder = bzip2_rs::DecoderReader::new(archive);
    let mut tar = tar::Archive::new(decoder);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if SPEECH_FILES.contains(&name) {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            write_atomic(&dir.join(name), &bytes)?;
        }
    }
    Ok(())
}

/// 途中で止まっても、壊れたファイルを完成品と間違えない
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let partial = path.with_extension("partial");
    fs::File::create(&partial)?.write_all(bytes)?;
    fs::rename(partial, path)
}
