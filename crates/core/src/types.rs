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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Enriched {
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
    /// 辞書が作り直されたことの通知。オーバーレイは `/api/glossary` を取り直す
    Glossary,
    Rank,
    /// 字幕の処理の状態（操作パネル用）
    Status,
    /// オーバーレイの見せ方
    Display,
    /// マイクの入力レベル（操作パネル用。字幕を動かしている間だけ流れる）
    Level,
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
    #[serde(rename = "glossary.updated", rename_all = "camelCase")]
    GlossaryUpdated { client_version: u32 },
    /// None はランクを表示しない（アカウントが外された）
    #[serde(rename = "rank.update")]
    RankUpdate { rank: Option<RankState> },
    /// 字幕を始められなかった理由。None なら問題なし
    #[serde(rename = "transcription.status")]
    TranscriptionStatus { error: Option<String> },
    #[serde(rename = "display", rename_all = "camelCase")]
    Display {
        subtitles: crate::settings::SubtitleDisplay,
        rank_language: LangCode,
        item_cards: ItemCardsView,
    },
    /// 直近のピーク（dBFS）。1 秒に 10 回ほど
    #[serde(rename = "audio.level")]
    AudioLevel { db: f32 },
}

impl ServerMsg {
    pub fn topic(&self) -> Topic {
        match self {
            Self::Snapshot { .. } | Self::SegmentUpsert { .. } => Topic::Subtitle,
            Self::GlossaryUpdated { .. } => Topic::Glossary,
            Self::RankUpdate { .. } => Topic::Rank,
            Self::TranscriptionStatus { .. } => Topic::Status,
            Self::Display { .. } => Topic::Display,
            Self::AudioLevel { .. } => Topic::Level,
        }
    }
}

/// 用語辞書（DESIGN.md 8.3）。`GET /api/glossary` で配信する。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Glossary {
    pub client_version: u32,
    /// 名前を取得した言語
    pub languages: Vec<LangCode>,
    /// キーは `"{kind}:{id}"`
    pub entities: BTreeMap<String, Entity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum EntityKind {
    Hero,
    Item,
    Ability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ItemSlot {
    Weapon,
    Vitality,
    Spirit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Entity {
    pub kind: EntityKind,
    pub class_name: String,
    pub names: BTreeMap<LangCode, String>,
    /// `names.ja` のふりがな（漢字を含む名前だけ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ruby: Option<Vec<RubySegment>>,
    #[serde(default)]
    pub aliases: BTreeMap<LangCode, Vec<String>>,
    /// `/assets/icons/...`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub slot: Option<ItemSlot>,
    /// スキルの持ち主（`"hero:{id}"`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hero: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RubySegment {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ruby: Option<String>,
}

/// 配信者の現在のランク（DESIGN.md 12 章）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RankState {
    /// `tier * 10 + subrank`
    pub badge: u32,
    pub tier: u32,
    pub subrank: u32,
    /// 累積のランクポイント（最後のランクマッチ終了時点）。分からなければ None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub points: Option<u32>,
    /// ティアの名前（言語ごと）
    pub names: BTreeMap<LangCode, String>,
    /// ティアの色（`#B47FEB` など）
    pub color: String,
    /// `/assets/rank/{badge}.webp`。アイコンをまだ取れていなければ None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub image: Option<String>,
    /// 今のサブランクの中での進み具合（ゲージ）。エターナスと未ランクは None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub progress: Option<RankProgress>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RankProgress {
    pub value: u32,
    pub max: u32,
}

/// アイテム説明のオーバーレイの見せ方
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ItemCardsView {
    pub hold_seconds: f32,
    /// 説明の言語（deadlock-api の言語名。例: `japanese`）
    pub language: String,
}

/// 操作パネルに返す設定。API キーはどこから読んだかだけ（キーそのものは返さない）
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct SettingsView {
    pub settings: crate::settings::Settings,
    pub keys: crate::secrets::KeyStatus,
    /// 設定を保存できない理由（変更はこの起動の間だけ効く）
    #[ts(optional)]
    pub unsaved: Option<String>,
}
