//! オーバーレイで選べるフォントの一覧と、そのファイルの配信（DESIGN.md 6.3）。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::UNIX_EPOCH;

use fontdb::{Database, FaceInfo, Source, Stretch, Style};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

use crate::settings::valid_font_name;

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

fn database() -> Database {
    let mut db = Database::new();
    db.load_system_fonts();
    db
}

fn scan() -> Vec<String> {
    let db = database();
    // 先頭は英語の名前。どの OS でも CSS の font-family に書ける
    entries(
        db.faces()
            .filter_map(|face| face.families.first())
            .map(|(family, _)| family.as_str()),
    )
}

/// PC のフォントを全部開くので別のスレッドで読む。入れたばかりのフォントも出るよう、毎回読み直す
pub async fn installed() -> Vec<String> {
    tokio::task::spawn_blocking(scan)
        .await
        .expect("blocking task")
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

/// そのファミリーの `@font-face`。一覧にないファミリーは `None`
pub async fn faces_css(family: String) -> Option<String> {
    tokio::task::spawn_blocking(move || css(&database(), &family))
        .await
        .expect("blocking task")
}

/// `faces_css` に書いた面のファイル
pub async fn face_file(family: String, postscript: String) -> Option<FaceFile> {
    tokio::task::spawn_blocking(move || file(&database(), &family, &postscript))
        .await
        .expect("blocking task")
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

    #[test]
    fn a_broken_collection_yields_nothing() {
        let data = collection(&[face("Probe", "Probe-Regular", 400)]);
        assert_eq!(extract(&data[..data.len() / 2], 0), None);
    }
}
