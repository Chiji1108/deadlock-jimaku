//! オーバーレイで選べるフォントの一覧（DESIGN.md 6.3）。

use serde::Serialize;
use tokio::sync::OnceCell;
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FontEntry {
    pub family: String,
    pub source: FontSource,
    /// すぐ使えるか
    pub ready: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FontSource {
    /// PC に入っている
    Installed,
}

/// 同じ名前（CSS は大文字と小文字を区別しない）は 1 つにまとめて名前順に並べる。
/// `.` で始まる名前は macOS の内部用で、選べない
fn entries<'a>(families: impl IntoIterator<Item = &'a str>) -> Vec<FontEntry> {
    let mut families: Vec<&str> = families
        .into_iter()
        .filter(|family| !family.is_empty() && !family.starts_with('.'))
        .collect();
    families.sort_by_cached_key(|family| family.to_lowercase());
    families.dedup_by_key(|family| family.to_lowercase());
    families
        .into_iter()
        .map(|family| FontEntry {
            family: family.to_owned(),
            source: FontSource::Installed,
            ready: true,
        })
        .collect()
}

fn scan() -> Vec<FontEntry> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    // 先頭は英語の名前。どの OS でも CSS の font-family に書ける
    entries(
        db.faces()
            .filter_map(|face| face.families.first())
            .map(|(family, _)| family.as_str()),
    )
}

/// PC のフォントを全部開くので、最初に呼ばれたときに 1 回だけ読む（入れたフォントは再起動で増える）
pub async fn installed() -> &'static [FontEntry] {
    static FONTS: OnceCell<Vec<FontEntry>> = OnceCell::const_new();
    FONTS
        .get_or_init(|| async {
            tokio::task::spawn_blocking(scan)
                .await
                .expect("blocking task")
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(families: &[&'static str]) -> Vec<String> {
        entries(families.iter().copied())
            .into_iter()
            .map(|e| e.family)
            .collect()
    }

    #[test]
    fn lists_each_family_once_in_name_order() {
        assert_eq!(
            names(&["Yu Gothic", "arial", "Meiryo", "Yu Gothic", "Arial"]),
            ["arial", "Meiryo", "Yu Gothic"]
        );
    }

    #[test]
    fn serializes_for_the_control_panel() {
        assert_eq!(
            serde_json::to_value(&entries(["Meiryo"])[0]).unwrap(),
            serde_json::json!({ "family": "Meiryo", "source": "installed", "ready": true })
        );
    }

    #[test]
    fn hides_families_that_cannot_be_chosen() {
        assert_eq!(names(&[".SF NS", "", "Helvetica"]), ["Helvetica"]);
    }
}
