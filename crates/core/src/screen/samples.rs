//! 見本にする画像（ヒーローのカード画像と、アイテムのショップの画像）を deadlock-api から取って、手元に置く
//! （DESIGN.md 18.2）。合わせて 10 MB ほどあるので、辞書のアイコンとは別に、画面を読み始めるときに取る。
//! `{root}/{clientVersion}/heroes/{id}.png` と `.../items/{id}.png` に置き、ほかの版の分は消す。

use std::path::Path;

use anyhow::Context;
use futures_util::{StreamExt, TryStreamExt, stream};
use image::RgbaImage;

use crate::assets::api::{ApiHero, ApiItem, Client};

/// 名前を取る言語。見本を見分けるのには `class_name` を使い、名前は評価の例が正解と比べるのに使う
const LANGUAGE: &str = "english";
/// 同時に取る数
const CONCURRENCY: usize = 8;

pub struct Sample {
    pub id: u32,
    pub class_name: String,
    /// 英語の名前
    pub name: String,
    pub image: RgbaImage,
}

pub struct Samples {
    pub client_version: u32,
    pub heroes: Vec<Sample>,
    pub items: Vec<Sample>,
}

/// 見本を読む。手元になければ取る。`version` が None なら、いちばん新しい版
pub async fn load(root: &Path, version: Option<u32>) -> anyhow::Result<Samples> {
    let client = Client::default();
    let version = match version {
        Some(version) => version,
        None => client.latest_client_version().await?,
    };
    let (heroes, items) = tokio::try_join!(
        client.heroes(LANGUAGE, version),
        client.items(LANGUAGE, version)
    )?;
    let dir = root.join(version.to_string());
    let (heroes, items) = tokio::try_join!(
        images(
            &client,
            dir.join("heroes"),
            heroes.into_iter().filter_map(hero)
        ),
        images(
            &client,
            dir.join("items"),
            items.into_iter().filter_map(item)
        ),
    )?;
    remove_other_versions(root, version).await;
    Ok(Samples {
        client_version: version,
        heroes,
        items,
    })
}

/// 画像を取る前の見本
struct Source {
    id: u32,
    class_name: String,
    name: String,
    url: String,
}

/// 選べるヒーローの、上部バーに出るカード画像
fn hero(hero: ApiHero) -> Option<Source> {
    if !hero.player_selectable || hero.disabled {
        return None;
    }
    let url = hero.images.get("icon_hero_card")?.as_str()?.to_owned();
    Some(Source {
        id: hero.id,
        name: hero.name?,
        class_name: hero.class_name,
        url,
    })
}

/// ショップで買えるアイテムの、アイテム欄に出る画像
fn item(item: ApiItem) -> Option<Source> {
    if item.kind != "upgrade" || !item.shopable {
        return None;
    }
    Some(Source {
        id: item.id,
        name: item.name?,
        class_name: item.class_name,
        url: item.shop_image?,
    })
}

async fn images(
    client: &Client,
    dir: std::path::PathBuf,
    sources: impl Iterator<Item = Source>,
) -> anyhow::Result<Vec<Sample>> {
    let dir = &dir;
    stream::iter(sources)
        .map(|source| async move {
            let path = dir.join(format!("{}.png", source.id));
            let bytes = match tokio::fs::read(&path).await {
                Ok(bytes) => bytes,
                Err(_) => {
                    let bytes = client.bytes(&source.url).await?;
                    crate::write_atomic(&path, &bytes).await?;
                    bytes
                }
            };
            // 読むのに時間が掛かるので、ほかの処理を止めない
            let image = tokio::task::spawn_blocking(move || image::load_from_memory(&bytes))
                .await?
                .with_context(|| format!("{}", path.display()))?;
            anyhow::Ok(Sample {
                id: source.id,
                class_name: source.class_name,
                name: source.name,
                image: image.into_rgba8(),
            })
        })
        .buffered(CONCURRENCY)
        .try_collect()
        .await
}

async fn remove_other_versions(root: &Path, version: u32) {
    let Ok(mut entries) = tokio::fs::read_dir(root).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_name().to_string_lossy() != version.to_string() {
            let _ = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_selectable_heroes_with_a_card_become_samples() {
        let hero = |value| hero(serde_json::from_value(value).unwrap());
        let doorman = json!({
            "id": 69, "class_name": "hero_doorman", "name": "The Doorman", "player_selectable": true,
            "images": { "icon_hero_card": "https://x/doorman_card.png" },
        });
        let found = hero(doorman.clone()).unwrap();
        assert_eq!((found.id, found.class_name.as_str()), (69, "hero_doorman"));
        assert_eq!(found.url, "https://x/doorman_card.png");

        let mut hidden = doorman.clone();
        hidden["player_selectable"] = json!(false);
        assert!(hero(hidden).is_none());
        let mut disabled = doorman.clone();
        disabled["disabled"] = json!(true);
        assert!(hero(disabled).is_none());
        let mut bare = doorman;
        bare["images"] = json!({});
        assert!(hero(bare).is_none());
    }

    #[test]
    fn only_items_sold_in_the_shop_become_samples() {
        let item = |value| item(serde_json::from_value(value).unwrap());
        let headhunter = json!({
            "id": 7, "class_name": "upgrade_headhunter", "name": "Headhunter", "type": "upgrade",
            "shopable": true, "shop_image": "https://x/headhunter.png",
        });
        assert_eq!(
            item(headhunter.clone()).unwrap().url,
            "https://x/headhunter.png"
        );

        let mut ability = headhunter.clone();
        ability["type"] = json!("ability");
        assert!(item(ability).is_none());
        let mut unsold = headhunter;
        unsold["shopable"] = json!(false);
        assert!(item(unsold).is_none());
    }
}
