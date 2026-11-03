//! deadlock-api の応答から用語辞書を組み立てる（DESIGN.md 8.2, 8.3）。通信もファイルも触らない。

use std::collections::{BTreeMap, HashMap};

use super::api::{ApiHero, ApiItem};
use crate::types::{Entity, EntityKind, Glossary, ItemSlot, LangCode};

/// 言語ごとの deadlock-api の応答。`en` は必須（どのエンティティを載せるかは英語版で決める）
pub struct Sources {
    pub heroes: BTreeMap<LangCode, Vec<ApiHero>>,
    pub items: BTreeMap<LangCode, Vec<ApiItem>>,
}

/// 日本語の別名（`data/aliases.ja.json`）。キーはエンティティの参照
pub type Aliases = BTreeMap<String, Vec<String>>;

/// ダウンロードするアイコン。`path` は icons ディレクトリからの相対パス
#[derive(Debug, PartialEq)]
pub struct Icon {
    /// このアイコンを使うエンティティの参照
    pub reference: String,
    pub url: String,
    pub path: String,
}

pub fn build(client_version: u32, sources: &Sources) -> (Glossary, Vec<Icon>) {
    let hero_names = names_by_lang(&sources.heroes, |h| {
        (h.id, h.name.as_deref(), &h.class_name)
    });
    let item_names = names_by_lang(&sources.items, |i| (i.id, i.name.as_deref(), &i.class_name));
    let base_heroes = sources
        .heroes
        .get("en")
        .map(Vec::as_slice)
        .unwrap_or_default();
    let base_items = sources
        .items
        .get("en")
        .map(Vec::as_slice)
        .unwrap_or_default();
    let items_by_class: HashMap<&str, &ApiItem> = base_items
        .iter()
        .map(|i| (i.class_name.as_str(), i))
        .collect();

    let mut entities = BTreeMap::new();
    let mut icons = Vec::new();
    let mut add = |kind: EntityKind, id: u32, entity: Entity, icon_url: Option<&str>| {
        let reference = format!("{}:{id}", kind_str(kind));
        // 同じスキルを複数のヒーローが持つことがあれば、最初のヒーローのものとして載せる
        if entities.contains_key(&reference) {
            return;
        }
        let mut entity = entity;
        if let Some(url) = icon_url {
            let path = format!("{}/{id}.{}", kind_str(kind), extension(url));
            // URL に版を含め、パッチでアイコンが変わっても古いキャッシュが残らないようにする
            entity.icon = Some(format!("/assets/{client_version}/icons/{path}"));
            icons.push(Icon {
                reference: reference.clone(),
                url: url.to_owned(),
                path,
            });
        }
        entities.insert(reference, entity);
    };

    // in_development では絞らない（最近リリースされたヒーローが true のまま残っている）
    let heroes = base_heroes
        .iter()
        .filter(|h| h.player_selectable && !h.disabled);
    for hero in heroes {
        let Some(names) = names_of(&hero_names, hero.id) else {
            continue;
        };
        let icon = ["icon_image_small_webp", "icon_image_small"]
            .iter()
            .find_map(|k| hero.images.get(*k)?.as_str());
        add(
            EntityKind::Hero,
            hero.id,
            entity(EntityKind::Hero, &hero.class_name, names),
            icon,
        );

        let abilities = (1..=4u8).filter_map(|n| {
            let class = hero.items.get(&format!("signature{n}"))?.as_deref()?;
            Some((n, items_by_class.get(class)?))
        });
        for (slot, ability) in abilities {
            let Some(names) = names_of(&item_names, ability.id) else {
                continue;
            };
            let mut e = entity(EntityKind::Ability, &ability.class_name, names);
            e.hero = Some(format!("hero:{}", hero.id));
            e.ability_slot = Some(slot);
            let icon = ability.image_webp.as_deref().or(ability.image.as_deref());
            add(EntityKind::Ability, ability.id, e, icon);
        }
    }

    for item in base_items
        .iter()
        .filter(|i| i.kind == "upgrade" && i.shopable)
    {
        let Some(names) = names_of(&item_names, item.id) else {
            continue;
        };
        let mut e = entity(EntityKind::Item, &item.class_name, names);
        e.slot = item.item_slot_type.as_deref().and_then(slot);
        let icon = [
            &item.shop_image_webp,
            &item.shop_image,
            &item.image_webp,
            &item.image,
        ]
        .into_iter()
        .find_map(Option::as_deref);
        add(EntityKind::Item, item.id, e, icon);
    }

    // 実際に名前が取れた言語だけを載せる（空の応答の言語をキャッシュ済みと扱わない）
    let languages = sources
        .heroes
        .keys()
        .filter(|lang| entities.values().any(|e| e.names.contains_key(*lang)))
        .cloned()
        .collect();
    let glossary = Glossary {
        client_version,
        languages,
        entities,
    };
    (glossary, icons)
}

/// 別名を重ねる。辞書には保存せず、配るたびに重ねる（アプリの更新で別名が変わっても、
/// Deadlock の更新を待たずに効く）
pub fn apply_aliases(glossary: &mut Glossary, aliases: &Aliases) {
    for (reference, aliases) in aliases {
        match glossary.entities.get_mut(reference) {
            Some(e) => {
                e.aliases.insert("ja".into(), aliases.clone());
            }
            None => tracing::warn!("aliases.ja.json: unknown entity {reference}"),
        }
    }
}

type NamesByLang<'a> = BTreeMap<&'a str, HashMap<u32, &'a str>>;

/// 名前が取れない（空、またはローカライズされていない class_name のまま）ものは除く
fn names_by_lang<'a, T>(
    by_lang: &'a BTreeMap<LangCode, Vec<T>>,
    fields: impl Fn(&'a T) -> (u32, Option<&'a str>, &'a String),
) -> NamesByLang<'a> {
    by_lang
        .iter()
        .map(|(lang, list)| {
            let names = list
                .iter()
                .filter_map(|x| {
                    let (id, name, class_name) = fields(x);
                    let name = name?.trim();
                    (!name.is_empty() && name != class_name).then_some((id, name))
                })
                .collect();
            (lang.as_str(), names)
        })
        .collect()
}

/// 英語名がないものは載せない
fn names_of(names: &NamesByLang, id: u32) -> Option<BTreeMap<LangCode, String>> {
    let out: BTreeMap<LangCode, String> = names
        .iter()
        .filter_map(|(lang, m)| Some(((*lang).to_owned(), (*m.get(&id)?).to_owned())))
        .collect();
    out.contains_key("en").then_some(out)
}

fn entity(kind: EntityKind, class_name: &str, names: BTreeMap<LangCode, String>) -> Entity {
    Entity {
        kind,
        class_name: class_name.to_owned(),
        names,
        ruby: None,
        aliases: BTreeMap::new(),
        icon: None,
        slot: None,
        hero: None,
        ability_slot: None,
    }
}

fn kind_str(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Hero => "hero",
        EntityKind::Item => "item",
        EntityKind::Ability => "ability",
    }
}

fn slot(s: &str) -> Option<ItemSlot> {
    match s {
        "weapon" => Some(ItemSlot::Weapon),
        "vitality" => Some(ItemSlot::Vitality),
        "spirit" => Some(ItemSlot::Spirit),
        _ => None,
    }
}

/// URL のパス部分の拡張子（クエリやフラグメントは見ない）
fn extension(url: &str) -> &str {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let file = path.rsplit('/').next().unwrap_or(path);
    file.rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("png")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hero(id: u32, class: &str, name: &str, selectable: bool) -> ApiHero {
        serde_json::from_value(json!({
            "id": id, "class_name": class, "name": name,
            "player_selectable": selectable, "disabled": false,
            "images": { "icon_image_small_webp": format!("https://x/{class}.webp") },
            "items": { "signature4": "ability_bullet_flurry", "weapon_primary": "w" },
        }))
        .unwrap()
    }

    fn item(id: u32, class: &str, name: &str, kind: &str) -> ApiItem {
        serde_json::from_value(json!({
            "id": id, "class_name": class, "name": name, "type": kind,
            "shopable": kind == "upgrade", "item_slot_type": "weapon",
            "image_webp": format!("https://x/{class}.webp"),
        }))
        .unwrap()
    }

    fn sources() -> Sources {
        Sources {
            heroes: BTreeMap::from([
                (
                    "en".into(),
                    vec![
                        hero(13, "hero_haze", "Haze", true),
                        hero(99, "hero_wip", "WIP", false),
                    ],
                ),
                ("ja".into(), vec![hero(13, "hero_haze", "ヘイズ", true)]),
            ]),
            items: BTreeMap::from([
                (
                    "en".into(),
                    vec![
                        item(1, "upgrade_clip_size", "Extended Magazine", "upgrade"),
                        item(2, "ability_bullet_flurry", "Bullet Dance", "ability"),
                        item(3, "ability_unused", "ability_unused", "ability"),
                    ],
                ),
                (
                    "ja".into(),
                    vec![
                        item(1, "upgrade_clip_size", "基礎拡張マガジン", "upgrade"),
                        item(2, "ability_bullet_flurry", "バレットダンス", "ability"),
                    ],
                ),
            ]),
        }
    }

    #[test]
    fn builds_heroes_abilities_and_shop_items() {
        let (g, icons) = build(6518, &sources());

        assert_eq!(
            g.entities.keys().collect::<Vec<_>>(),
            ["ability:2", "hero:13", "item:1"]
        );
        let haze = &g.entities["hero:13"];
        assert_eq!(haze.names["ja"], "ヘイズ");
        assert_eq!(
            haze.icon.as_deref(),
            Some("/assets/6518/icons/hero/13.webp")
        );
        assert_eq!(g.entities["ability:2"].hero.as_deref(), Some("hero:13"));
        assert_eq!(g.entities["item:1"].slot, Some(ItemSlot::Weapon));
        assert_eq!(icons.len(), 3);
        assert_eq!(g.languages, ["en", "ja"]);
    }

    #[test]
    fn shared_ability_is_listed_once() {
        let mut s = sources();
        let other = hero(14, "hero_other", "Other", true);
        s.heroes.get_mut("en").unwrap().push(other);
        let (g, icons) = build(6518, &s);

        assert_eq!(g.entities["ability:2"].hero.as_deref(), Some("hero:13"));
        assert_eq!(
            icons.iter().filter(|i| i.reference == "ability:2").count(),
            1
        );
    }

    #[test]
    fn languages_without_names_are_not_listed() {
        let mut s = sources();
        s.heroes.insert("th".into(), vec![]);
        s.items.insert("th".into(), vec![]);
        let (g, _) = build(6518, &s);

        assert_eq!(g.languages, ["en", "ja"]);
    }

    #[test]
    fn extension_ignores_query_and_dots_in_host() {
        assert_eq!(extension("https://cdn.example/a/haze.webp?v=2#x"), "webp");
        assert_eq!(extension("https://cdn.example/a/haze"), "png");
    }

    #[test]
    fn applies_aliases() {
        let (mut g, _) = build(6518, &sources());
        apply_aliases(
            &mut g,
            &Aliases::from([("item:1".into(), vec!["拡張マガジン".into()])]),
        );
        assert_eq!(g.entities["item:1"].aliases["ja"], ["拡張マガジン"]);
    }
}
