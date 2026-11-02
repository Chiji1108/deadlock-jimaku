//! 用語辞書とアイコンのキャッシュ（DESIGN.md 8.2）。
//!
//! `{root}/{CACHE_FORMAT}/{clientVersion}/glossary.json` と `.../{clientVersion}/icons/{kind}/{id}.{ext}` に置き、
//! アイコンは `/assets/{clientVersion}/icons/...` で配信する（URL に版を含めるので、長くキャッシュできる）。
//! 起動時に最新の clientVersion を確認し、キャッシュがなければ作る。通信できなければ手元の最新を使う。

mod api;
mod glossary;
mod readings;

pub use readings::ReadingModel;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use futures_util::{StreamExt, future::try_join_all, stream};

use crate::furigana::{self, Furigana};
use crate::lang;
use crate::store::Store;
use crate::types::{Glossary, LangCode};
use glossary::{Extras, Icon, Sources};

/// キャッシュの形式。Glossary の形やアイコンの置き方を変えたら上げる（古いキャッシュは読まない）
const CACHE_FORMAT: &str = "v4";

/// LLM で付けた読みのキャッシュ。名前ごとなので、パッチやキャッシュ形式をまたいで使い回す
const LLM_READINGS: &str = "readings-llm.ja.json";

/// 同時にダウンロードするアイコンの数
const ICON_CONCURRENCY: usize = 8;

#[derive(Clone)]
pub struct Loaded {
    pub glossary: Arc<Glossary>,
    /// clientVersion ごとのディレクトリを並べる場所
    pub root: PathBuf,
}

impl Loaded {
    pub fn icons_dir(&self, version: u32) -> PathBuf {
        version_dir(&self.root, version).join("icons")
    }
}

/// 辞書を用意して store に載せる。失敗してもアプリは字幕なしの辞書で動き続ける
pub async fn load_into(
    store: Arc<Store>,
    root: PathBuf,
    languages: Vec<LangCode>,
    reading_model: Option<ReadingModel>,
) {
    match load(&root, &languages, reading_model.as_ref()).await {
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

/// `reading_model` がなければ、辞書が知らない語も形態素解析の読みのままにする
pub async fn load(
    root: &Path,
    languages: &[LangCode],
    reading_model: Option<&ReadingModel>,
) -> anyhow::Result<Loaded> {
    let readings_cache = root.join(LLM_READINGS);
    let root = &root.join(CACHE_FORMAT);
    // どのエンティティを載せるかは英語版で決めるので、en は常に取る
    let languages = lang::normalize(languages.iter().cloned().chain(["en".to_owned()]));
    let client = api::Client::default();

    let built = async {
        let version = client.latest_client_version().await?;
        if let Some(cached) = read_cached(root, version, &languages) {
            return Ok(cached);
        }
        let readings = reading_model.map(|m| (m, readings_cache.as_path()));
        build_and_save(&client, root, version, &languages, readings).await
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
    readings: Option<(&ReadingModel, &Path)>,
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

    let extras = extras()?;
    // 名前の読みは形態素解析で付ける（辞書の読み込みと解析は同期処理なのでワーカーを塞がない）
    let (mut glossary, icons, uncertain) = tokio::task::spawn_blocking(move || {
        let (mut glossary, icons) = glossary::build(version, &sources, &extras);
        let furigana = Furigana::new()?;
        let uncertain = glossary::fill_readings(
            &mut glossary,
            |name| furigana.annotate(name),
            furigana::is_uncertain,
        );
        anyhow::Ok((glossary, icons, uncertain))
    })
    .await??;
    // 辞書が知らない語の読みだけ LLM に頼む。読めなかったら形態素解析の読みのまま
    let mut readings_missing = false;
    if let (Some((model, cache)), false) = (readings, uncertain.is_empty()) {
        let names: readings::Fallbacks = uncertain
            .iter()
            .filter_map(|r| {
                let e = glossary.entities.get(r)?;
                Some((
                    e.names.get("ja")?.clone(),
                    e.ruby.clone().unwrap_or_default(),
                ))
            })
            .collect();
        match readings::readings(model, cache, &names).await {
            Ok(found) => {
                for reference in &uncertain {
                    let Some(e) = glossary.entities.get_mut(reference) else {
                        continue;
                    };
                    if let Some(ruby) = e.names.get("ja").and_then(|n| found.get(n)) {
                        e.ruby = Some(ruby.clone());
                    }
                }
            }
            Err(e) => {
                tracing::warn!("readings: {e:#}");
                readings_missing = true;
            }
        }
    }
    let dir = version_dir(root, version);
    let failed = download_icons(client, &dir.join("icons"), icons).await;
    // 取れなかったアイコンは表示しない（404 を出さない）
    for reference in &failed {
        if let Some(e) = glossary.entities.get_mut(reference) {
            e.icon = None;
        }
    }

    // アイコンか読みが欠けていれば保存しない。次の起動で作り直し、欠けた分だけ取り直す
    if failed.is_empty() && !readings_missing {
        write_atomic(&dir.join("glossary.json"), &serde_json::to_vec(&glossary)?).await?;
    } else {
        tracing::warn!(
            "{} icons missing, readings missing: {readings_missing}; the glossary will be rebuilt next time",
            failed.len()
        );
    }
    Ok(Loaded {
        glossary: Arc::new(glossary),
        root: root.to_owned(),
    })
}

/// 取れなかったアイコンのエンティティの参照を返す
async fn download_icons(client: &api::Client, icons_dir: &Path, icons: Vec<Icon>) -> Vec<String> {
    stream::iter(icons)
        .map(|icon| async move {
            let path = icons_dir.join(&icon.path);
            if path.exists() {
                return None;
            }
            let saved = async { write_atomic(&path, &client.bytes(&icon.url).await?).await }.await;
            saved.err().map(|e| {
                tracing::warn!("icon {}: {e:#}", icon.url);
                icon.reference
            })
        })
        .buffer_unordered(ICON_CONCURRENCY)
        .filter_map(std::future::ready)
        .collect()
        .await
}

/// 途中で止まっても壊れたファイルが残らないよう、一時ファイルに書いてから置き換える
async fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    tokio::fs::create_dir_all(path.parent().expect("has parent")).await?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

fn version_dir(root: &Path, version: u32) -> PathBuf {
    root.join(version.to_string())
}

fn read_cached(root: &Path, version: u32, languages: &[LangCode]) -> Option<Loaded> {
    let loaded = read_version(root, version)?;
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
        .and_then(|version| read_version(root, version))
}

fn read_version(root: &Path, version: u32) -> Option<Loaded> {
    let json = std::fs::read(version_dir(root, version).join("glossary.json")).ok()?;
    let glossary = serde_json::from_slice(&json).ok()?;
    Some(Loaded {
        glossary: Arc::new(glossary),
        root: root.to_owned(),
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
            let dir = version_dir(root.path(), v);
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
