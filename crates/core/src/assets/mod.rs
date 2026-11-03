//! 用語辞書とアイコンのキャッシュ（DESIGN.md 8.2）。
//!
//! `{root}/{CACHE_FORMAT}/{clientVersion}/glossary.json` と `.../{clientVersion}/icons/{kind}/{id}.{ext}` に置き、
//! アイコンは `/assets/{clientVersion}/icons/...` で配信する（URL に版を含めるので、長くキャッシュできる）。
//! 起動時に最新の clientVersion を確認し、キャッシュがなければ作る。通信できなければ手元の最新を使う。

mod api;
mod glossary;
mod readings;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::{StreamExt, future::try_join_all, stream};

use crate::lang;
use crate::store::Store;
use crate::types::{Glossary, LangCode};
use glossary::{Aliases, Icon, Sources};

/// キャッシュの形式。Glossary の形やアイコンの置き方を変えたら上げる（古いキャッシュは読まない）
const CACHE_FORMAT: &str = "v8";

/// LLM で付けた読みのキャッシュ。名前ごとなので、パッチやキャッシュ形式をまたいで使い回す
const LLM_READINGS: &str = "readings-llm.ja.json";

/// 1 回の問い合わせで読ませる名前の数
const READINGS_PER_REQUEST: usize = 40;

/// 同時にダウンロードするアイコンの数
const ICON_CONCURRENCY: usize = 8;

/// 別名を配るサーバー（DESIGN.md 17 章）。アプリの更新を待たずに別名を足せる
const ALIASES_URL: &str = "https://api.deadlock.jp/obs/v1/aliases/ja";
/// 辞書の読み込みと同時に取りに行く。届かなければ同梱の別名で始める（起動を待たせすぎない）
const ALIASES_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub struct Loaded {
    pub glossary: Arc<Glossary>,
    /// 頼んだ言語。名前のない言語は `glossary.languages` に載らないので、どの設定で作った辞書かはこちらで見る
    pub requested: Vec<LangCode>,
    /// clientVersion ごとのディレクトリを並べる場所
    pub root: PathBuf,
}

impl Loaded {
    pub fn icons_dir(&self, version: u32) -> PathBuf {
        version_dir(&self.root, version).join("icons")
    }
}

/// 辞書を用意して store に載せる。失敗してもアプリは辞書なしで動き続ける。
/// 漢字名の読みは手書きの読みと、キャッシュ済みの LLM の読みを重ねて配信し、
/// 足りない名前は配信した後に問い合わせて配信し直す（`openai_key` がなければ手書きとキャッシュの読みだけ）
pub async fn load_into(store: Arc<Store>, root: PathBuf, openai_key: Option<String>) {
    let settings = store.settings();
    let languages = settings.glossary_languages();
    let (loaded, aliases) = tokio::join!(load(&root, &languages), aliases());
    let loaded = match loaded {
        Ok(loaded) => loaded,
        Err(e) => {
            tracing::error!("glossary unavailable: {e:#}");
            return;
        }
    };
    let manual = match manual_readings() {
        Ok(manual) => manual,
        Err(e) => {
            tracing::warn!("{e:#}");
            readings::Manual::new()
        }
    };
    let cache_path = root.join(LLM_READINGS);
    let cache = readings::load_cache(&cache_path).await;
    let publish = |cache: &readings::Cache| {
        let mut glossary = (*loaded.glossary).clone();
        glossary::apply_aliases(&mut glossary, &aliases);
        readings::apply(&mut glossary, cache, &manual);
        tracing::info!(
            "glossary {} ({} entities, {:?})",
            glossary.client_version,
            glossary.entities.len(),
            glossary.languages,
        );
        store.set_glossary(Loaded {
            glossary: Arc::new(glossary),
            requested: loaded.requested.clone(),
            root: loaded.root.clone(),
        });
    };
    let mut cache = match cache {
        Ok(cache) => cache,
        Err(e) => {
            // キャッシュを読めないまま問い合わせると、保存で上書きしてしまう
            tracing::warn!("readings cache unreadable, LLM readings are off: {e:#}");
            publish(&readings::Cache::new());
            return;
        }
    };
    publish(&cache);

    let Some(api_key) = openai_key else {
        return;
    };
    let model = readings::ReadingModel {
        api_key,
        model: settings.models.readings.clone(),
        reasoning_effort: settings.models.reasoning_effort.clone(),
        meter: store.meter(),
    };
    let missing = readings::missing(&loaded.glossary, &cache, &manual);
    if missing.is_empty() {
        return;
    }
    // 1 回の問い合わせを小さくし、届いた分ずつ保存する。失敗した分は次の起動で問い直す
    for names in missing.chunks(READINGS_PER_REQUEST) {
        match readings::ask(crate::http(), &model, names).await {
            Ok(found) => {
                tracing::info!("readings: {}/{} names", found.len(), names.len());
                cache.extend(found);
                if let Err(e) = readings::save_cache(&cache_path, &cache).await {
                    tracing::warn!("readings cache: {e:#}");
                }
            }
            Err(e) => tracing::warn!("readings: {e:#}"),
        }
    }
    publish(&cache);
}

pub async fn load(root: &Path, languages: &[LangCode]) -> anyhow::Result<Loaded> {
    let root = &root.join(CACHE_FORMAT);
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
    let loaded = match built.await {
        Ok(loaded) => loaded,
        Err(e) => {
            tracing::warn!("using cached glossary: {e:#}");
            newest_cached(root).ok_or(e)?
        }
    };
    Ok(Loaded {
        requested: languages,
        ..loaded
    })
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

    // 名前の読みは保存せず、読み込むたびに重ねる（`load_into`）
    let (mut glossary, icons) = glossary::build(version, &sources);
    let dir = version_dir(root, version);
    let failed = download_icons(client, &dir.join("icons"), icons).await;
    // 取れなかったアイコンは表示しない（404 を出さない）
    for reference in &failed {
        if let Some(e) = glossary.entities.get_mut(reference) {
            e.icon = None;
        }
    }

    // アイコンが欠けていれば保存しない。次の起動で作り直し、欠けた分だけ取り直す
    if failed.is_empty() {
        crate::write_atomic(&dir.join("glossary.json"), &serde_json::to_vec(&glossary)?).await?;
    } else {
        tracing::warn!(
            "{} icons missing; the glossary will be rebuilt next time",
            failed.len()
        );
    }
    Ok(Loaded {
        glossary: Arc::new(glossary),
        requested: Vec::new(),
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
            let saved =
                async { crate::write_atomic(&path, &client.bytes(&icon.url).await?).await }.await;
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
        requested: Vec::new(),
        root: root.to_owned(),
    })
}

/// サーバーの別名。取れなければ同梱の別名。どちらも辞書には保存せず、配るたびに重ねる
async fn aliases() -> Aliases {
    let fetched = async {
        let res = crate::http()
            .get(ALIASES_URL)
            .timeout(ALIASES_TIMEOUT)
            .send()
            .await?
            .error_for_status()?;
        anyhow::Ok(res.json::<Aliases>().await?)
    };
    match fetched.await {
        Ok(aliases) => aliases,
        Err(e) => {
            tracing::warn!("aliases from server: {e:#}; using the bundled ones");
            bundled_aliases().unwrap_or_else(|e| {
                tracing::warn!("{e:#}");
                Aliases::new()
            })
        }
    }
}

/// アプリに同梱の別名（サーバーに届かないとき）
fn bundled_aliases() -> anyhow::Result<Aliases> {
    serde_json::from_str(include_str!("../../../../data/aliases.ja.json"))
        .context("data/aliases.ja.json")
}

/// 手で書いた読み。辞書には保存せず、読み込むたびに重ねる（名前が変わっていたら使わない）
fn manual_readings() -> anyhow::Result<readings::Manual> {
    serde_json::from_str(include_str!("../../../../data/readings.ja.json"))
        .context("data/readings.ja.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_extras_parse() {
        bundled_aliases().unwrap();
        manual_readings().unwrap();
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
