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

use futures_util::{StreamExt, future::try_join_all, stream};

use crate::lang;
use crate::openai::Access;
use crate::store::Store;
use crate::types::{Glossary, LangCode};
use glossary::{Icon, Sources};

/// キャッシュの形式。Glossary の形やアイコンの置き方を変えたら上げる（古いキャッシュは読まない）
const CACHE_FORMAT: &str = "v9";

/// LLM で付けた読みのキャッシュ。名前ごとなので、パッチやキャッシュ形式をまたいで使い回す
const LLM_READINGS: &str = "readings-llm.ja.json";

/// 1 回の問い合わせで読ませる名前の数
const READINGS_PER_REQUEST: usize = 40;

/// 同時にダウンロードするアイコンの数
const ICON_CONCURRENCY: usize = 8;

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
/// 足りない名前は配信した後に問い合わせて配信し直す（`access` がなければ手書きとキャッシュの読みだけ）
pub async fn load_into(store: Arc<Store>, root: PathBuf, access: Option<Access>) {
    let settings = store.settings();
    let loaded = match load(&root, &settings.glossary_languages()).await {
        Ok(loaded) => loaded,
        Err(e) => {
            tracing::error!("glossary unavailable: {e:#}");
            return;
        }
    };
    let cache_path = root.join(LLM_READINGS);
    let cache = readings::load_cache(&cache_path).await;
    let publish = |cache: &readings::Cache| {
        let mut glossary = (*loaded.glossary).clone();
        readings::apply(&mut glossary, cache);
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
    // キャッシュを読めないまま問い合わせると、保存で上書きしてしまうので、LLM の読みは使わない
    let (mut cache, readable) = match cache {
        Ok(cache) => (cache, true),
        Err(e) => {
            tracing::warn!("readings cache unreadable, LLM readings are off: {e:#}");
            (readings::Cache::new(), false)
        }
    };
    publish(&cache);

    let (true, Some(access)) = (readable, access) else {
        return;
    };
    let model = readings::ReadingModel {
        access,
        model: settings.models.readings.clone(),
        reasoning_effort: settings.models.reasoning_effort.clone(),
        meter: store.meter(),
    };
    let missing = readings::missing(&loaded.glossary, &cache);
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

/// 字幕を始めるときに、新しい版の辞書を裏で確かめる（DESIGN.md 8.2）。トレイに常駐すると、
/// パッチが来ても起動し直さない。字幕の開始は待たせず、できたら差し替える
pub async fn watch_versions(store: Arc<Store>) {
    let mut settings = store.watch_settings();
    let mut enabled = settings.borrow_and_update().transcription_enabled;
    while settings.changed().await.is_ok() {
        let now = settings.borrow_and_update().transcription_enabled;
        let started = now && !enabled;
        enabled = now;
        if !started {
            continue;
        }
        match api::Client::default().latest_client_version().await {
            Ok(latest) if is_newer(latest, store.glossary().as_ref()) => store.refresh_glossary(),
            Ok(_) => {}
            Err(e) => tracing::warn!("client-versions: {e:#}"),
        }
    }
}

/// 辞書がなければ起動のときの処理に任せる
fn is_newer(latest: u32, current: Option<&Loaded>) -> bool {
    current.is_some_and(|loaded| loaded.glossary.client_version < latest)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_newer_version_rebuilds_the_glossary() {
        let loaded = |client_version| Loaded {
            glossary: Arc::new(Glossary {
                client_version,
                languages: Vec::new(),
                entities: Default::default(),
            }),
            requested: Vec::new(),
            root: PathBuf::new(),
        };
        assert!(is_newer(6519, Some(&loaded(6518))));
        assert!(!is_newer(6518, Some(&loaded(6518))));
        assert!(!is_newer(6517, Some(&loaded(6518))));
        assert!(!is_newer(6519, None));
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
