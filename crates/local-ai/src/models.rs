//! モデルを初回にダウンロードする。中身は SHA-256 で確かめる

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};

const SPEECH_URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17.tar.bz2";
const SPEECH_SHA256: &str = "7d1efa2138a65b0b488df37f8b89e3d91a60676e416f515b952358d83dfd347e";
const SPEECH_FILES: [&str; 2] = ["model.int8.onnx", "tokens.txt"];

/// Firefox の翻訳モデル（Bergamot）。日本語からは英語だけなので、ほかの言語は英語を経由する
const TRANSLATION_BASE: &str =
    "https://firefox-settings-attachments.cdn.mozilla.net/main-workspace/translations-models-v2";
const SPEECH_BYTES: u64 = 163_000_000;
const PAIR_BYTES: u64 = 35_000_000;

/// 添付ファイルの名前と、圧縮したものの SHA-256
type Attachment = (&'static str, &'static str);

struct Pair {
    name: &'static str,
    model: Attachment,
    source_vocabulary: Attachment,
    target_vocabulary: Attachment,
    shortlist: Attachment,
}

const JA_EN_VOCABULARY: Attachment = (
    "2566c4c6-4b9f-4d94-8898-38dbda231813.zst",
    "f41f32e36f879643a47fedbd0616fa12f416da62e23af8620ee4fc7705217acc",
);
const EN_RU_VOCABULARY: Attachment = (
    "13e4cd3b-1643-4fff-ac27-72c8732d1479.zst",
    "088a38d8b5faf6315b284e3088714f70a8c79438496b15826b6c62496503c447",
);
const PAIRS: [Pair; 4] = [
    Pair {
        name: "ja-en",
        model: (
            "53a52e41-5abb-4705-9ba8-8dbc07614bdc.zst",
            "c0c9f2e48979f022e16271049d96dbef9aa28f8fddc2d1db8fa0c399dec903f7",
        ),
        source_vocabulary: JA_EN_VOCABULARY,
        target_vocabulary: JA_EN_VOCABULARY,
        shortlist: (
            "3fe6eec6-c9fa-4262-85d1-a773ece70b6c.zst",
            "e6715f3e8a9d2ff08004b8185305de73906d7c31352f99a7f0424b4bf96a9422",
        ),
    },
    Pair {
        name: "en-ru",
        model: (
            "935dd36c-6429-4686-a3c3-074aa1873cf2.zst",
            "8a9ee496752361b7d7dd449dab046929880d21ab3ebcd2a261f076441d52dc30",
        ),
        source_vocabulary: EN_RU_VOCABULARY,
        target_vocabulary: EN_RU_VOCABULARY,
        shortlist: (
            "001d4c2d-b97f-454c-95c5-f02a436e4640.zst",
            "d5a33e067e7a7b2a1168f70d403424c4e68c539c333837bde05b150f83c39e8b",
        ),
    },
    Pair {
        name: "en-zh-Hans",
        model: (
            "321191b6-2c42-4bca-84ec-b7b074b67bd6.zst",
            "f0c96e546b0af7c1fe84647d450b1eba0b0591c167e1ead135b334e4b54f6e08",
        ),
        source_vocabulary: (
            "83278466-fec4-4944-a99d-c2372fa9de88.zst",
            "ec495ad8f0c80ba4eae5ac3b9a8e2ccb4a3a7becb1992efcfea5f4653ffdece1",
        ),
        target_vocabulary: (
            "84236654-1b66-42fc-9ba2-74c37f27e5eb.zst",
            "25c6c984a7556cb6478b9f1748ff68d8fd433c0e7d472bfdbd4d64a2dffa21f1",
        ),
        shortlist: (
            "900beaed-2dba-4a88-ad7c-852ec9e73f6a.zst",
            "c7e4b2ee202e6cf526d07f9c5a47b759ba32b0b42a5353d3db4ee1ff3bfb5217",
        ),
    },
    Pair {
        name: "en-zh-Hant",
        model: (
            "c21ec952-90bb-42dc-b245-7b73e90df282.zst",
            "8563ada56a7a07323979057d9185e69eec530f70dbc06c1001089cb3f4c01c9f",
        ),
        source_vocabulary: (
            "103d9764-f969-4c25-9ac9-ac08bc628e07.zst",
            "00736ec54a77532dd7c3f0c1473ed71503260465d67dc5c10d8096317ba932e4",
        ),
        target_vocabulary: (
            "349beae6-80c3-4e87-bfcb-4963d8f58882.zst",
            "41c1ce6c9e1fa4eb330dcef5abe718d2c32574d4c6a43dd37b2f22ad9907154b",
        ),
        shortlist: (
            "2fbf2d3e-9917-4564-8348-0bc0356de46c.zst",
            "efbff0226db207c2b698b711025e8a11c19ea66fc1cea816a6d182d2d3efff9d",
        ),
    },
];

/// 翻訳の 1 組のファイル
pub struct PairPaths {
    pub model: PathBuf,
    pub source_vocabulary: PathBuf,
    pub target_vocabulary: PathBuf,
    pub shortlist: PathBuf,
}

pub struct Paths {
    pub speech_model: PathBuf,
    pub tokens: PathBuf,
    /// `ja-en` は常にあり、`targets` の言語は `en-{言語}`
    pub pairs: Vec<(&'static str, PairPaths)>,
}

/// 足りないモデルをダウンロードする。`targets` は英語以外の翻訳先
pub fn ensure(
    dir: &Path,
    targets: &[String],
    mut progress: impl FnMut(u64, u64),
) -> anyhow::Result<Paths> {
    fs::create_dir_all(dir)?;
    let speech_model = dir.join(SPEECH_FILES[0]);
    let tokens = dir.join(SPEECH_FILES[1]);
    let selected: Vec<&Pair> = PAIRS
        .iter()
        .filter(|p| p.name == "ja-en" || targets.iter().any(|t| p.name == format!("en-{t}")))
        .collect();
    let total = SPEECH_BYTES + PAIR_BYTES * selected.len() as u64;
    let mut loaded = 0;
    if !(speech_model.exists() && tokens.exists()) {
        let archive = download(SPEECH_URL, SPEECH_SHA256, total, &mut loaded, &mut progress)?;
        extract_speech(&archive, dir)?;
    }
    let mut pairs = Vec::new();
    for pair in selected {
        let mut file =
            |attachment: Attachment| fetch(dir, attachment, total, &mut loaded, &mut progress);
        pairs.push((
            pair.name,
            PairPaths {
                model: file(pair.model)?,
                source_vocabulary: file(pair.source_vocabulary)?,
                target_vocabulary: file(pair.target_vocabulary)?,
                shortlist: file(pair.shortlist)?,
            },
        ));
    }
    Ok(Paths {
        speech_model,
        tokens,
        pairs,
    })
}

/// 展開したファイルの名前は、添付ファイルの名前から `.zst` を除いたもの
fn fetch(
    dir: &Path,
    (file, sha256): Attachment,
    total: u64,
    loaded: &mut u64,
    progress: &mut impl FnMut(u64, u64),
) -> anyhow::Result<PathBuf> {
    let target = dir.join(file.trim_end_matches(".zst"));
    if target.exists() {
        return Ok(target);
    }
    let url = format!("{TRANSLATION_BASE}/{file}");
    let compressed = download(&url, sha256, total, loaded, progress)?;
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(compressed.as_slice())?;
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes)?;
    write_atomic(&target, &bytes)?;
    Ok(target)
}

fn download(
    url: &str,
    sha256: &str,
    total: u64,
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
        progress(*loaded, total);
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
