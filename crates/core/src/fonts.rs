//! オーバーレイで選べるフォントの一覧、ファイルの配信、Google Fonts のフォントのダウンロード（DESIGN.md 6.3）。

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeSet, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, ensure};
use fontdb::{Database, FaceInfo, Source, Stretch, Style};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::time::timeout;
use ts_rs::TS;

use crate::settings::valid_font_name;

/// 一覧の 1 行。`languages` があるものはカタログの Google Fonts のフォントで、その言語の一覧に出す。`size_bytes` があるものは未取得。
/// `downloaded` は PC になくダウンロードで置いたフォントで、WebView が名前で描けない
#[derive(Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FontEntry {
    pub family: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub languages: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub size_bytes: Option<u64>,
    pub downloaded: bool,
}

/// `font-catalog.json` の形。frontend/scripts/font-samples.ts も同じファイルを読む
#[derive(Deserialize)]
struct Catalog {
    commit: String,
    fonts: Vec<CatalogFont>,
}

#[derive(Debug, Deserialize)]
pub struct CatalogFont {
    family: String,
    /// `google/fonts` の中のフォルダ
    dir: String,
    pub license: String,
    languages: Vec<String>,
    files: Vec<CatalogFile>,
}

#[derive(Debug, Deserialize)]
struct CatalogFile {
    name: String,
    size: u64,
    sha256: String,
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    serde_json::from_str(include_str!("font-catalog.json")).expect("valid font catalog")
});

impl CatalogFont {
    fn size_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }

    /// 途中で失敗して欠けたファミリーは、未取得のまま出して取り直せるようにする
    fn downloaded(&self, dir: &Path) -> bool {
        self.files.iter().all(|file| dir.join(&file.name).is_file())
    }
}

/// カタログの名前だけをダウンロードできる。URL は受け取らない
pub fn find(family: &str) -> Option<&'static CatalogFont> {
    CATALOG.fonts.iter().find(|font| font.family == family)
}

/// ダウンロードしたフォントの置き場所
fn folder(data_dir: &Path) -> PathBuf {
    data_dir.join("fonts")
}

/// 1 回のダウンロードの待ち時間
#[derive(Clone, Copy)]
struct Limits {
    /// ファミリーの全部のファイルを受け終えるまで
    total: Duration,
    /// 次のデータが届くまで。回線が切れたら `total` を待たずに止める
    stall: Duration,
}

const LIMITS: Limits = Limits {
    total: Duration::from_secs(600),
    stall: Duration::from_secs(30),
};

/// 全体の時間は `download_from` の 1 か所で切るので、`crate::http()` の 60 秒は持たない
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("valid client config")
    })
}

static DOWNLOADING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// ダウンロード中の印。途中で失敗しても `Drop` で外れる
struct Claim(String);

impl Claim {
    fn take(family: &str) -> Option<Self> {
        let mut running = DOWNLOADING.lock().expect("lock");
        running
            .insert(family.to_owned())
            .then(|| Self(family.to_owned()))
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        DOWNLOADING.lock().expect("lock").remove(&self.0);
    }
}

#[derive(Debug)]
pub enum DownloadError {
    Busy,
    Failed(anyhow::Error),
}

fn sha256(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 大きさと SHA-256 がカタログと一致したものだけを返す
async fn fetch(url: &str, file: &CatalogFile, limits: Limits) -> anyhow::Result<Vec<u8>> {
    let stalled = || format!("{} stalled", file.name);
    let mut res = timeout(limits.stall, client().get(url).send())
        .await
        .with_context(stalled)??
        .error_for_status()?;
    let mut body = Vec::with_capacity(file.size as usize);
    while let Some(chunk) = timeout(limits.stall, res.chunk())
        .await
        .with_context(stalled)??
    {
        body.extend_from_slice(&chunk);
        ensure!(body.len() as u64 <= file.size, "{} is too large", file.name);
    }
    ensure!(
        sha256(&body) == file.sha256,
        "{} has a wrong hash",
        file.name
    );
    Ok(body)
}

async fn fetch_all(
    origin: &str,
    font: &CatalogFont,
    limits: Limits,
) -> anyhow::Result<Vec<Vec<u8>>> {
    let mut bodies = Vec::with_capacity(font.files.len());
    for file in &font.files {
        let url = format!("{origin}/{}/{}", font.dir, file.name);
        bodies.push(fetch(&url, file, limits).await?);
    }
    Ok(bodies)
}

/// 全部のファイルが揃ってから置く。書き込みは途中で切らないので、待ち時間には含めない
async fn download_from(
    origin: &str,
    font: &CatalogFont,
    dir: &Path,
    limits: Limits,
) -> anyhow::Result<()> {
    let bodies = timeout(limits.total, fetch_all(origin, font, limits))
        .await
        .context("download took too long")??;
    for (file, body) in font.files.iter().zip(&bodies) {
        crate::write_atomic(&dir.join(&file.name), body).await?;
    }
    Ok(())
}

async fn download_once(
    origin: &str,
    font: &CatalogFont,
    dir: &Path,
    limits: Limits,
) -> Result<(), DownloadError> {
    let _claim = Claim::take(&font.family).ok_or(DownloadError::Busy)?;
    download_from(origin, font, dir, limits)
        .await
        .map_err(DownloadError::Failed)
}

/// 設定に保存できて、一覧に出すファミリーか。`.` で始まる名前は macOS の内部用で、選べない
fn selectable(family: &str) -> bool {
    valid_font_name(family) && !family.starts_with('.')
}

/// 設定に保存できる名前だけを、同じ名前（CSS は大文字と小文字を区別しない）は 1 つにまとめて
/// 名前順に並べる
fn entries<'a>(families: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut families: Vec<String> = families
        .into_iter()
        .filter(|family| selectable(family))
        .map(str::to_owned)
        .collect();
    families.sort_by_cached_key(|family| family.to_lowercase());
    families.dedup_by_key(|family| family.to_lowercase());
    families
}

fn database(dir: &Path) -> Database {
    let mut db = Database::new();
    db.load_system_fonts();
    db.load_fonts_dir(dir);
    db
}

/// PC にあるフォントはダウンロードを勧めない。カタログの行は PC のものでも Google Fonts に 1 回だけ出す。
/// カタログの順は勧める順なので保つ
fn list(pc: Vec<String>, catalog: &[CatalogFont], dir: &Path) -> Vec<FontEntry> {
    let lower = |family: &str| family.to_lowercase();
    let on_pc: HashSet<String> = pc.iter().map(|family| lower(family)).collect();
    let listed: HashSet<String> = catalog.iter().map(|font| lower(&font.family)).collect();
    catalog
        .iter()
        .map(|font| {
            let elsewhere = !on_pc.contains(&lower(&font.family));
            let saved = font.downloaded(dir);
            FontEntry {
                family: font.family.clone(),
                languages: Some(font.languages.clone()),
                size_bytes: (elsewhere && !saved).then(|| font.size_bytes()),
                downloaded: elsewhere && saved,
            }
        })
        .chain(
            pc.into_iter()
                .filter(|family| !listed.contains(&lower(family)))
                .map(|family| FontEntry {
                    family,
                    languages: None,
                    size_bytes: None,
                    downloaded: false,
                }),
        )
        .collect()
}

/// ダウンロードで置いたフォントは PC のフォントに数えない
fn scan(db: &Database, dir: &Path) -> Vec<FontEntry> {
    let downloaded =
        |face: &&FaceInfo| matches!(&face.source, Source::File(path) if path.starts_with(dir));
    // 先頭は英語の名前。どの OS でも CSS の font-family に書ける
    let pc = entries(
        db.faces()
            .filter(|face| !downloaded(face))
            .filter_map(|face| face.families.first())
            .map(|(family, _)| family.as_str()),
    );
    list(pc, &CATALOG.fonts, dir)
}

/// 配る面。URL から受け取った名前は `fontdb` の結果と比べるだけで、パスには使わない。
/// 直立で標準幅の面があればそれだけ、なければ全部の面が候補。`@font-face` は太さしか書かず、
/// 同じ太さの規則は後が前を隠すので、太さごとに通常の面に近い 1 面を、太さの順に残す
fn faces_of<'a>(db: &'a Database, family: &str) -> Vec<&'a FaceInfo> {
    let mut faces: Vec<&FaceInfo> = db
        .faces()
        .filter(|face| {
            face.families
                .first()
                .is_some_and(|(name, _)| selectable(name) && name.eq_ignore_ascii_case(family))
                && !face.post_script_name.is_empty()
        })
        .collect();
    let upright = |face: &FaceInfo| face.style == Style::Normal && face.stretch == Stretch::Normal;
    if faces.iter().any(|face| upright(face)) {
        faces.retain(|face| upright(face));
    }
    faces.sort_by_key(|&face| {
        (
            face.weight,
            face.style != Style::Normal,
            face.post_script_name.len(),
            &face.post_script_name,
        )
    });
    faces.dedup_by_key(|face| face.weight);
    faces
}

fn css(db: &Database, family: &str) -> Option<String> {
    let name = format!(r#""{}""#, family.replace('\\', r"\\").replace('"', r#"\""#));
    let rules: Vec<String> = faces_of(db, family)
        .into_iter()
        .filter_map(|face| {
            let axis = db.with_face_data(face.id, weight_axis)?;
            let (min, max) = axis.unwrap_or((face.weight.0, face.weight.0));
            // faces.css から見た相対の URL
            let file = utf8_percent_encode(&face.post_script_name, NON_ALPHANUMERIC);
            Some(format!(
                r#"@font-face {{ font-family: {name}; font-weight: {min} {max}; font-display: swap; src: url("{file}"); }}"#
            ))
        })
        .collect();
    (!rules.is_empty()).then(|| rules.join("\n"))
}

/// 1 面の sfnt。ローカルの配信で、組み直す面はどのみちメモリに載るので、どの面も同じ形で返す
pub struct FaceFile {
    pub etag: String,
    pub data: Vec<u8>,
}

/// ファイルの大きさと更新時刻で版を表す。URL が同じなら、入れ替えたフォントだけが別の版になる
fn etag(face: &FaceInfo) -> Option<String> {
    let Source::File(path) = &face.source else {
        return None;
    };
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(format!(
        r#""{:x}-{:x}-{:x}""#,
        meta.len(),
        modified.as_nanos(),
        face.index
    ))
}

/// 中身から作る版
pub fn content_etag(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!(r#""{:x}""#, hasher.finish())
}

fn file(db: &Database, family: &str, postscript: &str) -> Option<FaceFile> {
    let face = faces_of(db, family)
        .into_iter()
        .find(|face| face.post_script_name == postscript)?;
    let data = db.with_face_data(face.id, |data, index| {
        if data.starts_with(COLLECTION) {
            extract(data, index)
        } else {
            Some(data.to_vec())
        }
    })??;
    Some(FaceFile {
        etag: etag(face)?,
        data,
    })
}

/// PC のフォントとダウンロードしたフォントを読んだ `fontdb`。読み直すのは一覧を読むときと
/// ダウンロードを終えたときだけで、`faces.css` と面の配信は同じものを使う
pub struct Library {
    dir: PathBuf,
    db: Mutex<Option<Arc<Database>>>,
}

/// PC のフォントを全部開くので、別のスレッドで読む
async fn blocking<T: Send + 'static>(read: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(read)
        .await
        .expect("blocking task")
}

impl Library {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            dir: folder(data_dir),
            db: Mutex::new(None),
        }
    }

    /// 一覧をまだ読んでいなければ、ここで読む（OBS のオーバーレイは一覧を読まない）
    fn current(&self) -> Arc<Database> {
        let mut db = self.db.lock().expect("lock");
        db.get_or_insert_with(|| Arc::new(database(&self.dir)))
            .clone()
    }

    fn reload(&self) -> Arc<Database> {
        let db = Arc::new(database(&self.dir));
        *self.db.lock().expect("lock") = Some(db.clone());
        db
    }

    /// 入れたばかりのフォントも出るよう、読み直す
    pub async fn list(self: &Arc<Self>) -> Vec<FontEntry> {
        let library = self.clone();
        blocking(move || scan(&library.reload(), &library.dir)).await
    }

    /// そのファミリーの `@font-face`。一覧にないファミリーは `None`
    pub async fn faces_css(self: &Arc<Self>, family: String) -> Option<String> {
        let library = self.clone();
        blocking(move || css(&library.current(), &family)).await
    }

    /// `faces_css` に書いた面のファイル
    pub async fn face_file(
        self: &Arc<Self>,
        family: String,
        postscript: String,
    ) -> Option<FaceFile> {
        let library = self.clone();
        blocking(move || file(&library.current(), &family, &postscript)).await
    }

    /// 置いたフォントをすぐ配れるよう、読み直す
    pub async fn download(self: &Arc<Self>, font: &CatalogFont) -> Result<(), DownloadError> {
        let origin = format!(
            "https://raw.githubusercontent.com/google/fonts/{}",
            CATALOG.commit
        );
        download_once(&origin, font, &self.dir, LIMITS).await?;
        let library = self.clone();
        blocking(move || drop(library.reload())).await;
        Ok(())
    }
}

const COLLECTION: &[u8] = b"ttcf";

fn be16(data: &[u8], at: usize) -> Option<usize> {
    let bytes = data.get(at..at + 2)?;
    Some(u16::from_be_bytes(bytes.try_into().ok()?).into())
}

fn be32(data: &[u8], at: usize) -> Option<usize> {
    let bytes = data.get(at..at + 4)?;
    Some(u32::from_be_bytes(bytes.try_into().ok()?) as usize)
}

/// 面のテーブル一覧の位置
fn directory(data: &[u8], index: u32) -> Option<usize> {
    if data.starts_with(COLLECTION) {
        be32(data, 12 + 4 * index as usize)
    } else {
        Some(0)
    }
}

/// 可変フォントの `wght` 軸の範囲（`fvar`）。`None` は可変でないフォント
fn weight_axis(data: &[u8], index: u32) -> Option<(u16, u16)> {
    let dir = directory(data, index)?;
    let fvar = (0..be16(data, dir + 4)?)
        .map(|table| dir + 12 + 16 * table)
        .find(|&record| data.get(record..record + 4) == Some(b"fvar"))
        .and_then(|record| be32(data, record + 8))?;
    let axes = fvar + be16(data, fvar + 4)?;
    let size = be16(data, fvar + 10)?;
    let axis = (0..be16(data, fvar + 8)?)
        .map(|axis| axes + size * axis)
        .find(|&axis| data.get(axis..axis + 4) == Some(b"wght"))?;
    // 16.16 の固定小数点
    let value = |at| Some((be32(data, at)? >> 16) as u16);
    Some((value(axis + 4)?, value(axis + 12)?))
}

/// コレクションの 1 面を、ふつうの sfnt に組み直す。Chromium は `url(...#1)` の面の指定を無視する
fn extract(data: &[u8], index: u32) -> Option<Vec<u8>> {
    let dir = directory(data, index)?;
    let tables = be16(data, dir + 4)?;
    let mut out = data.get(dir..dir + 12 + 16 * tables)?.to_vec();
    for record in (12..).step_by(16).take(tables) {
        let start = be32(&out, record + 8)?;
        let end = start + be32(&out, record + 12)?;
        let moved = u32::try_from(out.len()).ok()?;
        out[record + 8..record + 12].copy_from_slice(&moved.to_be_bytes());
        out.extend_from_slice(data.get(start..end)?);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::time::{Duration, SystemTime};

    use super::*;

    #[test]
    fn lists_each_family_once_in_name_order() {
        assert_eq!(
            entries(["Yu Gothic", "arial", "Meiryo", "Yu Gothic", "Arial"]),
            ["arial", "Meiryo", "Yu Gothic"]
        );
    }

    #[test]
    fn hides_families_that_cannot_be_chosen() {
        let too_long = "a".repeat(257);
        assert_eq!(
            entries([".SF NS", "", "bad\nname", &too_long, "Helvetica"]),
            ["Helvetica"]
        );
    }

    #[test]
    fn keeps_names_with_quotes() {
        assert_eq!(entries(["Foo \"Bar\""]), ["Foo \"Bar\""]);
    }

    /// `fontdb` が読める最小の sfnt。テーブルは名前順に渡す
    fn sfnt(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut head = vec![0, 1, 0, 0];
        head.extend((tables.len() as u16).to_be_bytes());
        head.extend([0; 6]);
        let mut body = Vec::new();
        let start = 12 + 16 * tables.len();
        for (tag, table) in tables {
            head.extend(*tag);
            head.extend([0; 4]);
            head.extend(((start + body.len()) as u32).to_be_bytes());
            head.extend((table.len() as u32).to_be_bytes());
            body.extend(table);
            body.resize(body.len().next_multiple_of(4), 0);
        }
        head.extend(body);
        head
    }

    fn name_table(family: &str, postscript: &str) -> Vec<u8> {
        let encoded: Vec<Vec<u8>> = [family, postscript]
            .map(|text| text.encode_utf16().flat_map(u16::to_be_bytes).collect())
            .into();
        let mut table = vec![0, 0, 0, 2, 0, 30];
        let mut offset = 0u16;
        for (id, text) in [1u16, 6].iter().zip(&encoded) {
            for field in [3, 1, 0x409, *id, text.len() as u16, offset] {
                table.extend(u16::to_be_bytes(field));
            }
            offset += text.len() as u16;
        }
        table.extend(encoded.concat());
        table
    }

    fn os2(weight: u16, stretch: u16, italic: bool) -> Vec<u8> {
        let mut table = vec![0; 78];
        table[4..6].copy_from_slice(&weight.to_be_bytes());
        table[6..8].copy_from_slice(&stretch.to_be_bytes());
        table[63] = italic.into();
        table
    }

    fn fvar(axes: &[(&[u8; 4], u16, u16)]) -> Vec<u8> {
        let mut table = vec![0, 1, 0, 0, 0, 16, 0, 2];
        table.extend((axes.len() as u16).to_be_bytes());
        table.extend([0, 20, 0, 0, 0, 0]);
        for (tag, min, max) in axes {
            table.extend(*tag);
            for value in [*min, *min, *max] {
                table.extend(u32::from(value).wrapping_shl(16).to_be_bytes());
            }
            table.extend([0; 4]);
        }
        table
    }

    fn face(family: &str, postscript: &str, weight: u16) -> Vec<u8> {
        sfnt(&[
            (b"OS/2", os2(weight, 5, false)),
            (b"name", name_table(family, postscript)),
        ])
    }

    /// 面ごとにテーブルの位置をずらして 1 つのコレクションにする
    fn collection(faces: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"ttcf\0\x01\0\0".to_vec();
        out.extend((faces.len() as u32).to_be_bytes());
        let mut at = 12 + 4 * faces.len();
        for face in faces {
            out.extend((at as u32).to_be_bytes());
            at += face.len();
        }
        at = out.len();
        for face in faces {
            let mut face = face.clone();
            for record in (12..).step_by(16).take(be16(&face, 4).unwrap()) {
                let moved = be32(&face, record + 8).unwrap() + at;
                face[record + 8..record + 12].copy_from_slice(&(moved as u32).to_be_bytes());
            }
            at += face.len();
            out.extend(face);
        }
        out
    }

    /// 本物の読み込みと同じく、ファイルから読んだデータベース
    fn installed_fonts(files: &[(&str, Vec<u8>)]) -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        for (name, data) in files {
            std::fs::write(dir.path().join(name), data).unwrap();
        }
        let mut db = Database::new();
        db.load_fonts_dir(dir.path());
        (dir, db)
    }

    fn served(db: &Database, family: &str, postscript: &str) -> Option<Vec<u8>> {
        Some(file(db, family, postscript)?.data)
    }

    /// `faces.css` に書かれた面の名前
    fn sheet_files(db: &Database, family: &str) -> Vec<String> {
        css(db, family)
            .unwrap_or_default()
            .lines()
            .map(|rule| {
                let (_, file) = rule.split_once(r#"url(""#).unwrap();
                file.split_once('"').unwrap().0.replace("%2D", "-")
            })
            .collect()
    }

    fn styled(postscript: &str, weight: u16, stretch: u16, italic: bool) -> Vec<u8> {
        sfnt(&[
            (b"OS/2", os2(weight, stretch, italic)),
            (b"name", name_table("Probe", postscript)),
        ])
    }

    #[test]
    fn writes_the_upright_normal_width_faces() {
        let (_dir, db) = installed_fonts(&[
            ("regular.ttf", face("Probe", "Probe-Regular", 400)),
            ("bold.ttf", face("Probe", "Probe-Bold", 700)),
            ("italic.ttf", styled("Probe-Italic", 400, 5, true)),
            ("condensed.ttf", styled("Probe-Condensed", 400, 3, false)),
        ]);
        let css = css(&db, "probe").unwrap();
        assert_eq!(
            css.lines().collect::<Vec<_>>(),
            [
                r#"@font-face { font-family: "probe"; font-weight: 400 400; font-display: swap; src: url("Probe%2DRegular"); }"#,
                r#"@font-face { font-family: "probe"; font-weight: 700 700; font-display: swap; src: url("Probe%2DBold"); }"#,
            ]
        );
    }

    #[test]
    fn a_variable_font_declares_its_weight_range() {
        let (_dir, db) = installed_fonts(&[(
            "variable.ttf",
            sfnt(&[
                (b"OS/2", os2(400, 5, false)),
                (b"fvar", fvar(&[(b"opsz", 8, 144), (b"wght", 100, 900)])),
                (b"name", name_table("Probe", "Probe-Variable")),
            ]),
        )]);
        assert!(css(&db, "Probe").unwrap().contains("font-weight: 100 900;"));
    }

    #[test]
    fn only_the_weight_axis_counts_as_a_range() {
        let data = sfnt(&[(b"fvar", fvar(&[(b"opsz", 8, 144)]))]);
        assert_eq!(weight_axis(&data, 0), None);
    }

    #[test]
    fn escapes_the_family_name_in_the_sheet() {
        let name = r#"A"B\C"#;
        let (_dir, db) = installed_fonts(&[("a.ttf", face(name, "A/B", 400))]);
        let css = css(&db, name).unwrap();
        assert!(css.contains(r#"font-family: "A\"B\\C";"#), "{css}");
        assert!(css.contains(r#"url("A%2FB")"#), "{css}");
    }

    #[test]
    fn serves_only_faces_the_list_would_show() {
        let data = face("Probe", "Probe-Regular", 400);
        let (_dir, db) = installed_fonts(&[
            ("probe.ttf", data.clone()),
            ("hidden.ttf", face(".Hidden", "Hidden-Regular", 400)),
        ]);
        assert_eq!(served(&db, "Probe", "Probe-Regular"), Some(data));
        assert!(served(&db, "Probe", "Probe-Bold").is_none());
        assert!(served(&db, "Other", "Probe-Regular").is_none());
        assert!(served(&db, ".Hidden", "Hidden-Regular").is_none());
        assert_eq!(css(&db, ".Hidden"), None);
        assert_eq!(css(&db, "Other"), None);
    }

    #[test]
    fn a_path_in_the_url_reaches_no_file() {
        let (dir, db) = installed_fonts(&[("probe.ttf", face("Probe", "Probe-Regular", 400))]);
        let path = dir.path().join("probe.ttf");
        for postscript in [path.to_str().unwrap(), "../probe.ttf", "probe.ttf"] {
            assert!(served(&db, "Probe", postscript).is_none(), "{postscript}");
        }
        assert!(served(&db, path.to_str().unwrap(), "Probe-Regular").is_none());
    }

    #[test]
    fn keeps_the_regular_face_of_a_weight_whatever_the_load_order() {
        let files = [
            ("a.ttf", styled("Probe-Regular-Ornaments", 400, 5, false)),
            ("b.ttf", styled("Probe", 400, 5, false)),
            ("c.ttf", styled("Probe-Bold", 400, 5, false)),
            ("d.ttf", styled("Probe-Black", 900, 5, false)),
        ];
        let (_dir, forward) = installed_fonts(&files);
        let (_dir, backward) = installed_fonts(&[
            ("a.ttf", files[3].1.clone()),
            ("b.ttf", files[2].1.clone()),
            ("c.ttf", files[1].1.clone()),
            ("d.ttf", files[0].1.clone()),
        ]);
        for db in [&forward, &backward] {
            assert_eq!(sheet_files(db, "Probe"), ["Probe", "Probe-Black"]);
            assert!(served(db, "Probe", "Probe").is_some());
            assert!(served(db, "Probe", "Probe-Bold").is_none());
        }
    }

    #[test]
    fn names_of_the_same_length_are_broken_by_name() {
        let (_dir, db) = installed_fonts(&[
            ("a.ttf", styled("Probe-B", 400, 5, false)),
            ("b.ttf", styled("Probe-A", 400, 5, false)),
        ]);
        assert_eq!(sheet_files(&db, "Probe"), ["Probe-A"]);
    }

    #[test]
    fn a_family_of_only_condensed_faces_is_served() {
        let (_dir, db) = installed_fonts(&[
            ("regular.ttf", styled("Narrow-Regular", 400, 3, false)),
            ("italic.ttf", styled("Narrow-Italic", 400, 3, true)),
            ("bold.ttf", styled("Narrow-Bold", 700, 3, false)),
        ]);
        assert_eq!(sheet_files(&db, "Probe"), ["Narrow-Regular", "Narrow-Bold"]);
        assert!(served(&db, "Probe", "Narrow-Regular").is_some());
        assert!(served(&db, "Probe", "Narrow-Italic").is_none());
    }

    #[test]
    fn a_family_of_only_italic_faces_is_served() {
        let (_dir, db) = installed_fonts(&[("italic.ttf", styled("Probe-Italic", 400, 5, true))]);
        assert_eq!(sheet_files(&db, "Probe"), ["Probe-Italic"]);
        assert!(served(&db, "Probe", "Probe-Italic").is_some());
    }

    #[test]
    fn the_etag_follows_the_file() {
        let (dir, db) = installed_fonts(&[("probe.ttf", face("Probe", "Probe-Regular", 400))]);
        let path = dir.path().join("probe.ttf");
        let etag = || file(&db, "Probe", "Probe-Regular").unwrap().etag;
        let first = etag();
        assert!(first.starts_with('"') && first.ends_with('"'), "{first}");
        assert_eq!(first, etag());

        let touched = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(touched)
            .unwrap();
        let second = etag();
        assert_ne!(first, second);

        let longer = File::options().append(true).open(&path).unwrap();
        longer
            .set_len(longer.metadata().unwrap().len() + 4)
            .unwrap();
        longer.set_modified(touched).unwrap();
        assert_ne!(second, etag());
    }

    #[test]
    fn faces_of_a_collection_have_their_own_etag() {
        let (_dir, db) = installed_fonts(&[(
            "probe.ttc",
            collection(&[
                face("Probe", "Probe-Regular", 400),
                face("Probe", "Probe-Bold", 700),
            ]),
        )]);
        assert_ne!(
            file(&db, "Probe", "Probe-Regular").unwrap().etag,
            file(&db, "Probe", "Probe-Bold").unwrap().etag
        );
    }

    #[test]
    fn the_sheet_etag_follows_its_content() {
        assert_eq!(content_etag("a"), content_etag("a"));
        assert_ne!(content_etag("a"), content_etag("b"));
    }

    #[test]
    fn rebuilds_each_face_of_a_collection() {
        let regular = face("Probe", "Probe-Regular", 400);
        let bold = sfnt(&[
            (b"OS/2", os2(700, 5, false)),
            (b"fvar", fvar(&[(b"wght", 300, 800)])),
            (b"name", name_table("Probe", "Probe-Bold")),
        ]);
        let (_dir, db) =
            installed_fonts(&[("probe.ttc", collection(&[regular.clone(), bold.clone()]))]);
        assert_eq!(served(&db, "Probe", "Probe-Regular"), Some(regular));
        assert_eq!(served(&db, "Probe", "Probe-Bold"), Some(bold));
        assert!(css(&db, "Probe").unwrap().contains("font-weight: 300 800;"));
    }

    fn entry(family: &str, size_bytes: Option<u64>) -> FontEntry {
        FontEntry {
            family: family.to_owned(),
            languages: Some(vec!["ja".to_owned()]),
            size_bytes,
            downloaded: false,
        }
    }

    fn pc_entry(family: &str) -> FontEntry {
        FontEntry {
            languages: None,
            ..entry(family, None)
        }
    }

    fn catalog_font(family: &str, files: Vec<CatalogFile>) -> CatalogFont {
        CatalogFont {
            family: family.to_owned(),
            dir: "ofl/probe".to_owned(),
            license: "OFL-1.1".to_owned(),
            languages: vec!["ja".to_owned()],
            files,
        }
    }

    fn catalog_file(name: &str, body: &[u8]) -> CatalogFile {
        CatalogFile {
            name: name.to_owned(),
            size: body.len() as u64,
            sha256: sha256(body),
        }
    }

    #[test]
    fn a_font_on_the_pc_is_recommended_once_and_never_offered_for_download() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = [
            catalog_font("Probe", vec![catalog_file("a.ttf", b"12")]),
            catalog_font("Other", vec![catalog_file("b.ttf", b"1234")]),
        ];
        assert_eq!(
            list(
                vec!["probe".to_owned(), "Arial".to_owned()],
                &catalog,
                dir.path()
            ),
            [
                entry("Probe", None),
                entry("Other", Some(4)),
                pc_entry("Arial")
            ]
        );
    }

    #[test]
    fn only_a_font_missing_from_the_pc_is_marked_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = [
            catalog_font("On Pc", vec![catalog_file("a.ttf", b"1")]),
            catalog_font("Saved", vec![catalog_file("b.ttf", b"1")]),
        ];
        std::fs::write(dir.path().join("a.ttf"), b"1").unwrap();
        std::fs::write(dir.path().join("b.ttf"), b"1").unwrap();
        assert_eq!(
            list(vec!["On Pc".to_owned()], &catalog, dir.path()),
            [
                entry("On Pc", None),
                FontEntry {
                    downloaded: true,
                    ..entry("Saved", None)
                }
            ]
        );
    }

    #[test]
    fn a_family_with_a_missing_file_can_be_downloaded_again() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = [catalog_font(
            "Probe",
            vec![catalog_file("a.ttf", b"12"), catalog_file("b.ttf", b"345")],
        )];
        std::fs::write(dir.path().join("a.ttf"), b"12").unwrap();
        assert_eq!(
            list(vec![], &catalog, dir.path()),
            [entry("Probe", Some(5))]
        );
        std::fs::write(dir.path().join("b.ttf"), b"345").unwrap();
        assert_eq!(
            list(vec![], &catalog, dir.path()),
            [FontEntry {
                downloaded: true,
                ..entry("Probe", None)
            }]
        );
    }

    /// 偽の配布元。`gate` があれば、それが開くまで応答を止める
    async fn fake_origin(
        files: Vec<(&'static str, Vec<u8>)>,
        gate: Option<std::sync::Arc<tokio::sync::Semaphore>>,
    ) -> String {
        use axum::extract::Path as UrlPath;
        let files = std::sync::Arc::new(files);
        let app = axum::Router::new().route(
            "/{*path}",
            axum::routing::get(move |UrlPath(path): UrlPath<String>| {
                let (files, gate) = (files.clone(), gate.clone());
                async move {
                    if let Some(gate) = gate {
                        let _open = gate.acquire().await.unwrap();
                    }
                    let found = files.iter().find(|(name, _)| path.ends_with(name));
                    found
                        .map(|(_, body)| body.clone())
                        .ok_or(axum::http::StatusCode::NOT_FOUND)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        format!("http://{addr}")
    }

    /// 4 バイトのうち 2 バイトを送ったところで黙る配布元
    async fn stalling_origin() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = socket.read(&mut [0; 1024]).await.unwrap();
            assert!(request > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nfo")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        format!("http://{addr}")
    }

    fn saved(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|files| {
                let mut names: Vec<String> = files
                    .map(|file| file.unwrap().file_name().into_string().unwrap())
                    .collect();
                names.sort();
                names
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn saves_every_file_of_a_family_and_serves_it() {
        let regular = face("Download Probe", "Probe-Regular", 400);
        let bold = face("Download Probe", "Probe-Bold", 700);
        let font = catalog_font(
            "Download Probe",
            vec![
                catalog_file("r[wght].ttf", &regular),
                catalog_file("b.ttf", &bold),
            ],
        );
        let origin = fake_origin(vec![("r[wght].ttf", regular), ("b.ttf", bold)], None).await;
        let dir = tempfile::tempdir().unwrap();
        let fonts = dir.path().join("fonts");
        download_once(&origin, &font, &fonts, LIMITS).await.unwrap();
        assert_eq!(saved(&fonts), ["b.ttf", "r[wght].ttf"]);
        assert_eq!(
            sheet_files(&database(&fonts), "Download Probe"),
            ["Probe-Regular", "Probe-Bold"]
        );
    }

    #[tokio::test]
    async fn a_saved_font_is_served_once_the_list_is_read_again() {
        let data = tempfile::tempdir().unwrap();
        let library = Arc::new(Library::new(data.path()));
        let family = || "Later Probe".to_owned();
        assert_eq!(library.faces_css(family()).await, None);

        let fonts = folder(data.path());
        std::fs::create_dir(&fonts).unwrap();
        std::fs::write(
            fonts.join("later.ttf"),
            face("Later Probe", "Later-Regular", 400),
        )
        .unwrap();
        assert_eq!(library.faces_css(family()).await, None);

        let list = library.list().await;
        assert!(list.iter().all(|font| font.family != "Later Probe"));
        assert!(library.faces_css(family()).await.is_some());
        assert!(
            library
                .face_file(family(), "Later-Regular".to_owned())
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn a_wrong_hash_saves_nothing_of_the_family() {
        let font = catalog_font(
            "Hash Probe",
            vec![
                catalog_file("good.ttf", b"good"),
                catalog_file("bad.ttf", b"real"),
            ],
        );
        let origin = fake_origin(
            vec![
                ("good.ttf", b"good".to_vec()),
                ("bad.ttf", b"evil".to_vec()),
            ],
            None,
        )
        .await;
        let dir = tempfile::tempdir().unwrap();
        let result = download_once(&origin, &font, dir.path(), LIMITS).await;
        assert!(matches!(result, Err(DownloadError::Failed(_))));
        assert!(saved(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn a_file_larger_than_the_catalog_says_is_refused() {
        let font = catalog_font("Size Probe", vec![catalog_file("big.ttf", b"1234")]);
        let origin = fake_origin(vec![("big.ttf", vec![0; 1 << 20])], None).await;
        let dir = tempfile::tempdir().unwrap();
        let result = download_once(&origin, &font, dir.path(), LIMITS).await;
        assert!(matches!(result, Err(DownloadError::Failed(_))));
        assert!(saved(dir.path()).is_empty());
    }

    async fn failure(origin: &str, family: &str, limits: Limits) -> String {
        let font = catalog_font(family, vec![catalog_file("f.ttf", b"font")]);
        let dir = tempfile::tempdir().unwrap();
        let result = download_once(origin, &font, dir.path(), limits).await;
        assert!(saved(dir.path()).is_empty());
        match result {
            Err(DownloadError::Failed(e)) => e.to_string(),
            other => panic!("{other:?}"),
        }
    }

    const SHORT: Duration = Duration::from_millis(100);

    #[tokio::test]
    async fn a_body_that_stops_arriving_is_given_up_before_the_total_limit() {
        let limits = Limits {
            total: Duration::from_secs(30),
            stall: SHORT,
        };
        let origin = stalling_origin().await;
        assert_eq!(
            failure(&origin, "Stall Probe", limits).await,
            "f.ttf stalled"
        );
    }

    #[tokio::test]
    async fn a_server_that_never_answers_is_given_up_before_the_total_limit() {
        let limits = Limits {
            total: Duration::from_secs(30),
            stall: SHORT,
        };
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        let origin = fake_origin(vec![], Some(gate)).await;
        assert_eq!(
            failure(&origin, "Silent Probe", limits).await,
            "f.ttf stalled"
        );
    }

    #[tokio::test]
    async fn the_total_limit_covers_the_whole_download() {
        let limits = Limits {
            total: SHORT,
            stall: Duration::from_secs(30),
        };
        let origin = stalling_origin().await;
        assert_eq!(
            failure(&origin, "Total Probe", limits).await,
            "download took too long"
        );
    }

    #[tokio::test]
    async fn the_same_font_is_downloaded_once_at_a_time() {
        let font = catalog_font("Busy Probe", vec![catalog_file("f.ttf", b"font")]);
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        let origin = fake_origin(vec![("f.ttf", b"font".to_vec())], Some(gate.clone())).await;
        let dir = tempfile::tempdir().unwrap();

        let first = download_once(&origin, &font, dir.path(), LIMITS);
        let second = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let result = download_once(&origin, &font, dir.path(), LIMITS).await;
            gate.add_permits(1);
            result
        };
        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        assert!(matches!(second, Err(DownloadError::Busy)));

        gate.add_permits(1);
        download_once(&origin, &font, dir.path(), LIMITS)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_failed_download_can_be_retried() {
        let font = catalog_font("Retry Probe", vec![catalog_file("f.ttf", b"font")]);
        let origin = fake_origin(vec![("other.ttf", vec![])], None).await;
        let dir = tempfile::tempdir().unwrap();
        for _ in 0..2 {
            let result = download_once(&origin, &font, dir.path(), LIMITS).await;
            assert!(matches!(result, Err(DownloadError::Failed(_))));
        }
    }

    #[test]
    fn the_catalog_holds_only_open_font_license_fonts_for_known_languages() {
        let mut families = HashSet::new();
        for font in &CATALOG.fonts {
            assert!(selectable(&font.family), "{}", font.family);
            assert!(families.insert(&font.family), "{}", font.family);
            assert_eq!(font.license, "OFL-1.1", "{}", font.family);
            assert!(font.dir.starts_with("ofl/"), "{}", font.family);
            assert!(!font.files.is_empty(), "{}", font.family);
            for language in &font.languages {
                assert!(crate::lang::find(language).is_some(), "{language}");
            }
            for file in &font.files {
                assert!(!file.name.contains(['/', '\\']), "{}", file.name);
                assert_eq!(file.sha256.len(), 64, "{}", file.name);
            }
        }
    }

    /// ダウンロードしたファイルは 1 つのフォルダに置き、ファイルの有無で取得済みかを見る。
    /// 大文字と小文字を区別しないファイルシステムでも、ファミリーをまたいで重ならない
    #[test]
    fn file_names_are_unique_across_the_catalog() {
        let mut names = HashSet::new();
        for file in CATALOG.fonts.iter().flat_map(|font| &font.files) {
            assert!(names.insert(file.name.to_lowercase()), "{}", file.name);
        }
    }

    fn recommended(language: &str) -> Vec<&'static str> {
        CATALOG
            .fonts
            .iter()
            .filter(|font| font.languages.iter().any(|code| code == language))
            .map(|font| font.family.as_str())
            .collect()
    }

    #[test]
    fn every_language_has_recommended_fonts() {
        for language in crate::lang::LANGUAGES {
            let count = recommended(language.code).len();
            assert!(count >= 3, "{}: {count}", language.code);
            let few_fonts = matches!(language.code, "ko" | "zh-Hans" | "zh-Hant" | "th");
            assert!(count <= 5 || !few_fonts, "{}", language.code);
        }
    }

    #[test]
    fn recommendations_start_rounded_and_end_with_the_body_font() {
        for (language, first, last) in [
            ("ja", "Zen Maru Gothic", "Noto Sans JP"),
            ("en", "Nunito", "Noto Sans"),
        ] {
            let families = recommended(language);
            assert_eq!(families.first(), Some(&first));
            assert_eq!(families.last(), Some(&last));
        }
    }

    #[test]
    fn the_notices_name_every_catalog_font() {
        let notices: HashSet<&str> = include_str!("../../../notices/fonts.txt").lines().collect();
        for font in &CATALOG.fonts {
            assert!(notices.contains(font.family.as_str()), "{}", font.family);
        }
    }

    #[test]
    fn a_broken_collection_yields_nothing() {
        let data = collection(&[face("Probe", "Probe-Regular", 400)]);
        assert_eq!(extract(&data[..data.len() / 2], 0), None);
    }
}
