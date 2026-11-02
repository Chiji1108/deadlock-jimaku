//! フロントと共有する型。`cargo test -p overlays-core` で TS に書き出す（DESIGN.md 7, 9, 10 章）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// BCP 47 の言語コード（DESIGN.md 6 章）。対応表は Phase 1 で入れる。
pub type LangCode = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Stage {
    Partial,
    Final,
    Enriched,
    Discarded,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "t", rename_all = "lowercase")]
#[ts(export)]
pub enum Token {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        ruby: Option<String>,
    },
    Entity {
        /// `"{kind}:{id}"`（例: `hero:13`）
        #[serde(rename = "ref")]
        reference: String,
        /// 実際に発話された表現（略称など）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        spoken: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum EnrichedKind {
    Speech,
    Noise,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Enriched {
    pub kind: EnrichedKind,
    pub source: Vec<Token>,
    pub translations: BTreeMap<LangCode, Vec<Token>>,
}

/// 1 発話 = 1 Segment。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Segment {
    pub id: String,
    /// epoch ms。表示順のキー
    #[ts(type = "number")]
    pub started_at: u64,
    pub stage: Stage,
    pub lang: LangCode,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub enriched: Option<Enriched>,
    /// 表示時間の起点（epoch ms）。enriched になった時刻、または enrich が失敗した時刻
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub settled_at: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Topic {
    Subtitle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "lowercase")]
#[ts(export)]
pub enum ClientMsg {
    Subscribe { topics: Vec<Topic> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum ServerMsg {
    /// 接続直後に送る
    #[serde(rename = "snapshot")]
    Snapshot { segments: Vec<Segment> },
    #[serde(rename = "segment.upsert")]
    SegmentUpsert { segment: Segment },
}

impl ServerMsg {
    pub fn topic(&self) -> Topic {
        match self {
            Self::Snapshot { .. } | Self::SegmentUpsert { .. } => Topic::Subtitle,
        }
    }
}
