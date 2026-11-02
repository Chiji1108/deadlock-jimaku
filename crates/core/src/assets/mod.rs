//! 用語辞書とアイコンのキャッシュ（DESIGN.md 8.2）。
//!
//! `{root}/{clientVersion}/glossary.json` と `{root}/{clientVersion}/icons/{kind}/{id}.{ext}` に置く。
//! 起動時に最新の clientVersion を確認し、キャッシュがなければ作る。通信できなければ手元の最新を使う。

mod api;
mod glossary;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use futures_util::{StreamExt, future::try_join_all, stream};

use crate::lang;
use crate::store::Store;
use crate::types::{Glossary, LangCode};
use glossary::{Extras, Icon, Sources};

/// 同時にダウンロードするアイコンの数
const ICON_CONCURRENCY: usize = 8;

#[derive(Clone)]
pub struct Loaded {
    pub glossary: Arc<Glossary>,
    /// clientVersion ごとのディレクトリ（`icons/` を含む）
    pub dir: PathBuf,
}

impl Loaded {
    pub fn icons_dir(&self) -> PathBuf {
        self.dir.join("icons")
    }
}

/// 辞書を用意して store に載せる。失敗してもアプリは字幕なしの辞書で動き続ける
pub async fn load_into(store: Arc<Store>, root: PathBuf, languages: Vec<LangCode>) {
    match load(&root, &languages).await {
        Ok(loaded) => {
            tracing::info!(
                "glossary {} ({} entities, {:?})",
                loaded.glossary.client_version,
                loaded.glossary.entities.len(),
                loaded.glossary.languages,
            );
            store.set_glossary(loaded);
        }
        Err(e) => tracing::error!("glossary unavailable: {e:#}"),
    }
}

pub async fn load(root: &Path, languages: &[LangCode]) -> anyhow::Result<Loaded> {
    // どのエンティティを載せるかは英語版で決めるので、en は常に取る
    let languages = lang::normalize(languages.iter().cloned().chain(["en".to_owned()]));
    let client = api::Client::default();

    let built = async {
        let version = client.latest_client_version().await?;
        if let Some(cached) = read_cached(root, version, &languages) {
            return Ok(cached);
        }
        build_and_save(&client, root, version, &languages).await
    };
    match built.await {
        Ok(loaded) => Ok(loaded),
        Err(e) => {
            tracing::warn!("using cached glossary: {e:#}");
            newest_cached(root).ok_or(e)
        }
    }
}

async fn build_and_save(
    client: &api::Client,
    root: &Path,
    version: u32,
    languages: &[LangCode],
) -> anyhow::Result<Loaded> {
    tracing::info!("building glossary {version} for {languages:?}");
    let fetch = |code: &LangCode| {
        let api_name = lang::find(code).expect("normalized").api;
        let code = code.clone();
        async move {
            let (heroes, items) = tokio::try_join!(
                client.heroes(api_name, version),
                client.items(api_name, version)
            )?;
            anyhow::Ok((code, heroes, items))
        }
    };
    let mut sources = Sources {
        heroes: BTreeMap::new(),
        items: BTreeMap::new(),
    };
    for (code, heroes, items) in try_join_all(languages.iter().map(fetch)).await? {
        sources.heroes.insert(code.clone(), heroes);
        sources.items.insert(code, items);
    }

    let (glossary, icons) = glossary::build(version, &sources, &extras()?);
    let dir = root.join(version.to_string());
    download_icons(client, &dir.join("icons"), icons).await;

    let json = serde_json::to_vec(&glossary)?;
    tokio::fs::create_dir_all(&dir).await?;
    let tmp = dir.join("glossary.json.tmp");
    tokio::fs::write(&tmp, json).await?;
    tokio::fs::rename(&tmp, dir.join("glossary.json")).await?;
    Ok(Loaded {
        glossary: Arc::new(glossary),
        dir,
    })
}

/// 取れなかったアイコンは警告だけ出して、アイコンなしで表示する
async fn download_icons(client: &api::Client, icons_dir: &Path, icons: Vec<Icon>) {
    stream::iter(icons)
        .for_each_concurrent(ICON_CONCURRENCY, |icon| async move {
            let path = icons_dir.join(&icon.path);
            if path.exists() {
                return;
            }
            let saved = async {
                tokio::fs::create_dir_all(path.parent().expect("has parent")).await?;
                let bytes = client.bytes(&icon.url).await?;
                tokio::fs::write(&path, bytes).await?;
                anyhow::Ok(())
            }
            .await;
            if let Err(e) = saved {
                tracing::warn!("icon {}: {e:#}", icon.url);
            }
        })
        .await;
}

fn read_cached(root: &Path, version: u32, languages: &[LangCode]) -> Option<Loaded> {
    let loaded = read_dir(&root.join(version.to_string()))?;
    let covers = languages
        .iter()
        .all(|l| loaded.glossary.languages.contains(l));
    covers.then_some(loaded)
}

/// 言語が足りなくても、ないよりはましなので使う
fn newest_cached(root: &Path) -> Option<Loaded> {
    std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .max()
        .and_then(|version| read_dir(&root.join(version.to_string())))
}

fn read_dir(dir: &Path) -> Option<Loaded> {
    let json = std::fs::read(dir.join("glossary.json")).ok()?;
    let glossary = serde_json::from_slice(&json).ok()?;
    Some(Loaded {
        glossary: Arc::new(glossary),
        dir: dir.to_owned(),
    })
}

fn extras() -> anyhow::Result<Extras> {
    Ok(Extras {
        aliases_ja: serde_json::from_str(include_str!("../../../../data/aliases.ja.json"))
            .context("data/aliases.ja.json")?,
        readings_ja: serde_json::from_str(include_str!("../../../../data/readings.ja.json"))
            .context("data/readings.ja.json")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_extras_parse() {
        extras().unwrap();
    }

    #[test]
    fn newest_cached_picks_highest_version() {
        let root = tempfile::tempdir().unwrap();
        for v in [6500u32, 6518, 6510] {
            let dir = root.path().join(v.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            let g = Glossary {
                client_version: v,
                languages: vec!["en".into()],
                entities: BTreeMap::new(),
            };
            std::fs::write(dir.join("glossary.json"), serde_json::to_vec(&g).unwrap()).unwrap();
        }
        std::fs::create_dir_all(root.path().join("not-a-version")).unwrap();

        let loaded = newest_cached(root.path()).unwrap();
        assert_eq!(loaded.glossary.client_version, 6518);
        assert!(read_cached(root.path(), 6510, &["en".into()]).is_some());
        assert!(read_cached(root.path(), 6510, &["ja".into()]).is_none());
    }
}
