//! 字幕の統計。確定した字幕から、喋った量と、呼んだヒーロー・アイテム・スキルの回数を数える。
//! 文そのものは残さず、数だけを `stats.json` に保存する（`monthly`）。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use ts_rs::TS;

use crate::monthly::{Monthly, Periods};
use crate::types::{Segment, Token};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Stats {
    /// 確定した字幕の数
    #[ts(type = "number")]
    pub utterances: u64,
    /// 空白を除いた文字数（日本語や中国語で見せる）
    #[ts(type = "number")]
    pub characters: u64,
    /// 空白で区切った語の数（英語などで見せる）
    #[ts(type = "number")]
    pub words: u64,
    /// `"hero:13"` → 呼んだ回数。ヒーロー・アイテム・スキルだけ
    #[ts(type = "Record<string, number>")]
    pub names: BTreeMap<String, u64>,
}

pub type StatsView = Periods<Stats>;

/// 数える用語の種類。人と基本用語は数えない
const COUNTED: &[&str] = &["hero:", "item:", "ability:"];

pub struct Tally(Monthly<Stats>);

impl Tally {
    /// `path` が読めなければ今月の分を 0 から数える（壊れていても止めない）
    pub fn new(path: PathBuf) -> Self {
        Self(Monthly::new(path))
    }

    /// 確定した字幕を 1 つ数える
    pub fn count(&self, seg: &Segment) {
        let tokens = seg
            .enriched
            .as_ref()
            .map(|e| e.source.as_slice())
            .or(seg.tokens.as_deref())
            .unwrap_or_default();
        let names: Vec<&str> = tokens
            .iter()
            .filter_map(|t| match t {
                Token::Entity { reference, .. } => Some(reference.as_str()),
                Token::Text { .. } => None,
            })
            .filter(|r| COUNTED.iter().any(|kind| r.starts_with(kind)))
            .collect();
        let characters = seg.text.chars().filter(|c| !c.is_whitespace()).count() as u64;
        let words = seg.text.split_whitespace().count() as u64;
        self.0.add(|stats| {
            stats.utterances += 1;
            stats.characters += characters;
            stats.words += words;
            for name in &names {
                *stats.names.entry((*name).to_owned()).or_default() += 1;
            }
        });
    }

    pub fn watch(&self) -> watch::Receiver<StatsView> {
        self.0.watch()
    }

    pub async fn persist(&self) {
        self.0.persist().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Enriched, Stage};

    fn entity(r: &str) -> Token {
        Token::Entity {
            reference: r.to_owned(),
            spoken: None,
            ruby: None,
        }
    }

    fn text(t: &str) -> Token {
        Token::Text {
            text: t.to_owned(),
            ruby: None,
        }
    }

    fn segment(text: &str, source: Vec<Token>) -> Segment {
        Segment {
            id: "mic-1-1".to_owned(),
            started_at: 1,
            stage: Stage::Enriched,
            lang: "ja".to_owned(),
            text: text.to_owned(),
            enriched: Some(Enriched {
                source,
                translations: Default::default(),
            }),
            tokens: None,
            draft: None,
            settled_at: Some(2),
        }
    }

    #[test]
    fn counts_speech_and_game_names_but_not_people() {
        let dir = tempfile::tempdir().unwrap();
        let tally = Tally::new(dir.path().join("stats.json"));
        tally.count(&segment(
            "ヘイズ の ウルト、ヘイズ強い",
            vec![
                entity("hero:13"),
                text("の"),
                entity("ability:731943444"),
                text("、"),
                entity("hero:13"),
                entity("person:me"),
                entity("term:soul"),
                text("強い"),
            ],
        ));
        let view = tally.watch().borrow().clone();
        let s = &view.session;
        assert_eq!(s.utterances, 1);
        assert_eq!(s.characters, 13);
        assert_eq!(s.words, 3);
        assert_eq!(
            s.names,
            BTreeMap::from([
                ("ability:731943444".to_owned(), 1),
                ("hero:13".to_owned(), 2)
            ])
        );
        assert_eq!(view.month, view.session);
    }
}
