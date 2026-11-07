//! オーバーレイで選べるフォントの一覧（DESIGN.md 6.3）。

use crate::settings::valid_font_name;

/// 設定に保存できる名前だけを、同じ名前（CSS は大文字と小文字を区別しない）は 1 つにまとめて
/// 名前順に並べる。`.` で始まる名前は macOS の内部用で、選べない
fn entries<'a>(families: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut families: Vec<String> = families
        .into_iter()
        .filter(|family| valid_font_name(family) && !family.starts_with('.'))
        .map(str::to_owned)
        .collect();
    families.sort_by_cached_key(|family| family.to_lowercase());
    families.dedup_by_key(|family| family.to_lowercase());
    families
}

fn scan() -> Vec<String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
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

#[cfg(test)]
mod tests {
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
}
