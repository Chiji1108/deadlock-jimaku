//! 人の辞書（DESIGN.md 9.12）。配信者（自分）と、よく一緒に遊ぶ人の、名前・呼ばれ方・アイコン。
//!
//! 用語の辞書に「人」として足す（`apply`）。そうすると、文字起こしへのヒント、字幕での強調とアイコン、
//! 翻訳での名前（その言語の名前に置き換える）が、ヒーローやアイテムと同じ仕組みで効く。
//! この PC の設定とデータフォルダにだけ置く（サーバーには集めない）。名前は、選んだモードによっては、
//! 文字起こしや判定のサービスに送られる（用語のヒントと同じ）

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::types::{Entity, EntityKind, Glossary, LangCode};

/// 覚えられる人数（用語のヒントには上限があるので、増やしすぎない）
pub const MAX_PEOPLE: usize = 50;
/// 名前と呼ばれ方の長さの上限（字）
const MAX_NAME_CHARS: usize = 40;
/// 1 人の呼ばれ方の数の上限
const MAX_ALIASES: usize = 10;
/// アイコンの大きさの上限
const MAX_ICON_BYTES: usize = 4 * 1024 * 1024;
/// アイコンを取りに行くときの待ち時間
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// 辞書の 1 人
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Person {
    /// 用語の参照（`person:{id}`）に使う。英小文字と数字
    pub id: String,
    /// 配信者本人
    pub me: bool,
    /// 言語ごとの名前（ふつうは日本語と英語）
    pub names: BTreeMap<LangCode, String>,
    /// 呼ばれ方（あだ名や、名前の一部）。話す言語で書く
    pub aliases: Vec<String>,
    /// アイコンのファイルの名前（データフォルダの `people/` の中）
    #[ts(optional)]
    pub icon: Option<String>,
}

impl Person {
    pub fn reference(&self) -> String {
        format!("person:{}", self.id)
    }
}

/// アイコンのファイルの名前として安全（英数字と `.` `-` だけ。フォルダを抜けない）
fn safe_file(name: &str) -> bool {
    let plain = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-';
    !name.is_empty() && name.len() <= 64 && !name.starts_with('.') && name.chars().all(plain)
}

/// 操作パネルから届いた一覧を、保存できる形に整える。空の名前や呼ばれ方は捨てる。
/// 名前が 1 つもない人、同じ id、2 人目の「自分」は、エラーにして何も変えない
pub fn sanitized(people: Vec<Person>) -> Result<Vec<Person>, String> {
    if people.len() > MAX_PEOPLE {
        return Err(format!("too many people (at most {MAX_PEOPLE})"));
    }
    let short = |text: &str| -> String { text.trim().chars().take(MAX_NAME_CHARS).collect() };
    let mut ids = BTreeSet::new();
    let mut cleaned = Vec::new();
    for person in people {
        let plain = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
        if person.id.is_empty() || person.id.len() > 32 || !person.id.chars().all(plain) {
            return Err(format!("bad person id: {:?}", person.id));
        }
        if !ids.insert(person.id.clone()) {
            return Err(format!("duplicate person id: {}", person.id));
        }
        let names: BTreeMap<LangCode, String> = (person.names.iter())
            .map(|(lang, name)| (lang.clone(), short(name)))
            .filter(|(_, name)| !name.is_empty())
            .collect();
        if names.is_empty() {
            return Err("a person needs a name".into());
        }
        let mut aliases: Vec<String> = Vec::new();
        for alias in person.aliases.iter().map(|alias| short(alias)) {
            if !alias.is_empty() && !aliases.contains(&alias) && aliases.len() < MAX_ALIASES {
                aliases.push(alias);
            }
        }
        if person.icon.as_deref().is_some_and(|file| !safe_file(file)) {
            return Err("bad icon file name".into());
        }
        cleaned.push(Person {
            names,
            aliases,
            ..person
        });
    }
    if cleaned.iter().filter(|person| person.me).count() > 1 {
        return Err("only one person can be me".into());
    }
    Ok(cleaned)
}

/// 用語の辞書に、人を足す。辞書の言語に名前がなければ、ある名前で埋める（翻訳でも名前が出るように）
pub fn apply(glossary: &mut Glossary, people: &[Person]) {
    for person in people {
        // 埋めるのに使う名前。英語、日本語、ほかの順
        let known = ["en", "ja"].iter().find_map(|lang| person.names.get(*lang));
        let Some(fallback) = known.or_else(|| person.names.values().next()) else {
            continue;
        };
        let mut names = person.names.clone();
        let languages = glossary.languages.iter().map(String::as_str);
        for lang in languages.chain(["en"]) {
            if !names.contains_key(lang) {
                names.insert(lang.to_owned(), fallback.clone());
            }
        }
        let entity = Entity {
            kind: EntityKind::Person,
            class_name: person.id.clone(),
            names,
            ruby: None,
            icon: person.icon.as_ref().map(|file| format!("/people/{file}")),
            slot: None,
            hero: None,
            ability_slot: None,
            aliases: person.aliases.clone(),
        };
        glossary.entities.insert(person.reference(), entity);
    }
}

/// アイコンを置くフォルダ
pub fn icons_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("people")
}

/// アイコンのファイルの場所。名前が安全でなければ None
pub fn icon_path(data_dir: &Path, file: &str) -> Option<PathBuf> {
    safe_file(file).then(|| icons_dir(data_dir).join(file))
}

/// 画像の種類（先頭の数バイトで見分ける）。知らない形式は受け付けない
fn extension(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("jpg"),
        [b'G', b'I', b'F', b'8', ..] => Some("gif"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("webp"),
        _ => None,
    }
}

/// アイコンを保存して、ファイルの名前を返す。名前は中身から作る（同じ画像は同じ名前。差し替えたら名前が変わるので、
/// 画面は古い画像を出し続けない）
pub async fn save_icon(data_dir: &Path, bytes: &[u8]) -> anyhow::Result<String> {
    ensure!(bytes.len() <= MAX_ICON_BYTES, "the image is too large");
    let extension = extension(bytes).context("not a PNG, JPEG, GIF or WebP image")?;
    let digest: String = (Sha256::digest(bytes).iter().take(8))
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let file = format!("{digest}.{extension}");
    let dir = icons_dir(data_dir);
    tokio::fs::create_dir_all(&dir).await?;
    tokio::fs::write(dir.join(&file), bytes).await?;
    Ok(file)
}

/// 使われなくなったアイコンを消す
pub async fn prune_icons(data_dir: &Path, people: &[Person]) {
    let used: BTreeSet<&str> = people.iter().filter_map(|p| p.icon.as_deref()).collect();
    let Ok(mut entries) = tokio::fs::read_dir(icons_dir(data_dir)).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        if !name.to_str().is_some_and(|name| used.contains(name)) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

/// 取ってきたアイコンと、分かれば名前
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct Fetched {
    /// 保存したファイルの名前
    pub icon: String,
    /// Steam の表示名（Steam から取ったとき）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
}

/// ページのアイコンを読める場所。チャンネルのページに、アイコンの画像の場所（`og:image`）が書いてある
const PAGE_HOSTS: [&str; 5] = [
    "twitch.tv",
    "www.twitch.tv",
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
];

/// ブラウザのふりをしないと、チャンネルのページをくれないことがある
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// Twitch か YouTube のチャンネルの URL から、アイコンを取って保存する
pub async fn fetch_from_page(data_dir: &Path, url: &str) -> anyhow::Result<Fetched> {
    let page = reqwest::Url::parse(url.trim()).context("not a URL")?;
    let host = page.host_str().unwrap_or_default();
    ensure!(
        page.scheme() == "https" && PAGE_HOSTS.contains(&host),
        "only Twitch and YouTube channel pages are supported"
    );
    let res = crate::http()
        .get(page)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en")
        .timeout(FETCH_TIMEOUT)
        .send()
        .await?
        .error_for_status()?;
    let html = res.text().await?;
    let image = og_image(&html).context("the page has no icon")?;
    let icon = download(data_dir, &image).await?;
    Ok(Fetched { icon, name: None })
}

/// ページの `<meta property="og:image" content="…">` の中身
fn og_image(html: &str) -> Option<String> {
    let at = html.find("property=\"og:image\"")?;
    // 同じタグの中の content（前後どちらに書いてあってもよい）
    let start = html[..at].rfind('<')?;
    let end = at + html[at..].find('>')?;
    let tag = &html[start..end];
    let content = tag.split("content=\"").nth(1)?.split('"').next()?;
    Some(content.replace("&amp;", "&"))
}

/// Steam のアカウント（ランクの表示につないであるもの）のアイコンと表示名を、deadlock-api から取って保存する
pub async fn fetch_from_steam(data_dir: &Path, account: u32) -> anyhow::Result<Fetched> {
    let url = format!("https://api.deadlock-api.com/v1/players/steam?account_ids={account}");
    let res = crate::http().get(url).timeout(FETCH_TIMEOUT).send().await?;
    let profiles: serde_json::Value = res.error_for_status()?.json().await?;
    let profile = &profiles[0];
    let text = |key: &str| profile[key].as_str().filter(|text| !text.is_empty());
    let image = text("avatarfull")
        .or(text("avatarmedium"))
        .or(text("avatar"));
    let icon = download(data_dir, image.context("the Steam profile has no icon")?).await?;
    Ok(Fetched {
        icon,
        name: text("personaname").map(str::to_owned),
    })
}

async fn download(data_dir: &Path, image: &str) -> anyhow::Result<String> {
    let image = reqwest::Url::parse(image).context("the icon is not a URL")?;
    ensure!(image.scheme() == "https", "the icon is not on https");
    let res = crate::http()
        .get(image)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await?
        .error_for_status()?;
    if res
        .content_length()
        .is_some_and(|n| n as usize > MAX_ICON_BYTES)
    {
        bail!("the image is too large");
    }
    save_icon(data_dir, &res.bytes().await?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(id: &str, names: &[(&str, &str)], aliases: &[&str]) -> Person {
        Person {
            id: id.into(),
            names: (names.iter())
                .map(|(lang, name)| ((*lang).to_owned(), (*name).to_owned()))
                .collect(),
            aliases: aliases.iter().map(|alias| (*alias).to_owned()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn people_are_cleaned_before_saving() {
        let me = Person {
            me: true,
            ..person(
                "a1",
                &[("ja", " ちじ "), ("en", "Chiji"), ("ko", " ")],
                &["ちーさん", "", "ちーさん"],
            )
        };
        let cleaned = sanitized(vec![me.clone()]).unwrap();
        assert_eq!(cleaned[0].names.len(), 2);
        assert_eq!(cleaned[0].names["ja"], "ちじ");
        assert_eq!(cleaned[0].aliases, ["ちーさん"]);

        let nameless = person("b2", &[("ja", "  ")], &[]);
        assert!(sanitized(vec![nameless]).is_err());
        assert!(sanitized(vec![me.clone(), me.clone()]).is_err());
        let second = Person {
            id: "c3".into(),
            ..me.clone()
        };
        assert!(sanitized(vec![me.clone(), second]).is_err());
        assert!(sanitized(vec![person("A B", &[("ja", "x")], &[])]).is_err());
        let escaping = Person {
            icon: Some("../settings.json".into()),
            ..me
        };
        assert!(sanitized(vec![escaping]).is_err());
    }

    #[test]
    fn a_person_joins_the_glossary_with_a_name_in_every_language() {
        let mut glossary = Glossary {
            client_version: 1,
            languages: vec!["ja".into(), "en".into(), "ru".into()],
            entities: BTreeMap::new(),
        };
        let friend = Person {
            icon: Some("abcd.png".into()),
            ..person("f1", &[("ja", "みり")], &["みりちゃん"])
        };
        apply(&mut glossary, &[friend]);
        let entity = &glossary.entities["person:f1"];
        assert_eq!(entity.kind, EntityKind::Person);
        // 名前のない言語は、ある名前で埋める
        assert_eq!(
            (entity.names["en"].as_str(), entity.names["ru"].as_str()),
            ("みり", "みり")
        );
        assert_eq!(entity.icon.as_deref(), Some("/people/abcd.png"));
        assert_eq!(entity.aliases, ["みりちゃん"]);
    }

    #[test]
    fn the_icon_of_a_channel_page_is_read_from_its_meta_tag() {
        let twitch = r#"<head><meta property="og:image" content="https://static-cdn.jtvnw.net/jtv_user_pictures/x-profile_image-300x300.png"/></head>"#;
        assert_eq!(
            og_image(twitch).unwrap(),
            "https://static-cdn.jtvnw.net/jtv_user_pictures/x-profile_image-300x300.png"
        );
        let youtube = r#"<meta content="https://yt3.googleusercontent.com/abc=s900&amp;x=1" property="og:image">"#;
        assert_eq!(
            og_image(youtube).unwrap(),
            "https://yt3.googleusercontent.com/abc=s900&x=1"
        );
        assert_eq!(og_image("<html></html>"), None);
    }

    #[tokio::test]
    async fn icons_are_named_by_their_content_and_pruned_when_unused() {
        let dir = tempfile::tempdir().unwrap();
        let png = [0x89, b'P', b'N', b'G', 1, 2, 3];
        let file = save_icon(dir.path(), &png).await.unwrap();
        assert!(file.ends_with(".png") && safe_file(&file));
        assert_eq!(save_icon(dir.path(), &png).await.unwrap(), file);
        assert!(save_icon(dir.path(), b"<svg></svg>").await.is_err());
        assert!(icon_path(dir.path(), "../x").is_none());
        let kept = Person {
            icon: Some(file.clone()),
            ..person("a", &[("ja", "x")], &[])
        };
        let other = save_icon(dir.path(), &[0xFF, 0xD8, 0xFF, 9]).await.unwrap();
        prune_icons(dir.path(), &[kept]).await;
        assert!(icon_path(dir.path(), &file).unwrap().exists());
        assert!(!icon_path(dir.path(), &other).unwrap().exists());
    }
}
