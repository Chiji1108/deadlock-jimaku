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
        /// 実際に発話された表現（略称など）。原文ではこれをそのまま出す（翻訳では公式名を出す）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        spoken: Option<String>,
        /// 発話された表現のふりがな（漢字を含むときだけ）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        ruby: Option<Vec<RubySegment>>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Enriched {
    pub source: Vec<Token>,
    pub translations: Translations,
}

/// 言語ごとの訳文
pub type Translations = BTreeMap<LangCode, Vec<Token>>;

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
    /// PC の中で作った、ふりがなと用語の付いた文。`enriched` のあとは、途中から付いていた印に残す
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub draft_source: Option<Vec<Token>>,
    /// 書きかけの訳文。`enriched` が届いたら消す
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub draft: Option<Translations>,
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
    /// OpenAI の使用量と概算の料金（操作パネル用）
    Usage,
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
    /// 字幕を始められなかった・止まった理由。None なら問題なし
    #[serde(rename = "transcription.status")]
    TranscriptionStatus { error: Option<TranscriptionError> },
    #[serde(rename = "display", rename_all = "camelCase")]
    Display {
        subtitles: crate::settings::SubtitleDisplay,
        rank_language: LangCode,
        rank_content: crate::settings::RankContent,
        rank_anchor: crate::settings::Anchor,
        item_cards: ItemCardsView,
    },
    /// 直近のピーク（dBFS）。1 秒に 10 回ほど
    #[serde(rename = "audio.level")]
    AudioLevel { db: f32 },
    #[serde(rename = "usage")]
    Usage { usage: crate::usage::UsageView },
    /// ローカル AI の状態。None は動いていない
    #[serde(rename = "live.status")]
    LiveStatus { status: Option<LiveStatus> },
    /// 操作パネル以外（OBS の字幕オーバーレイ、トレイ）で設定が変わった。パネルは取り直す
    #[serde(rename = "settings.changed")]
    SettingsChanged,
}

/// ローカル AI の状態（DESIGN.md 9.6）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LiveState {
    /// モデルをダウンロードしている
    Preparing,
    /// 聞いている
    Listening,
    /// 起動できない
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LiveStatus {
    pub state: LiveState,
    /// 利用者に見せる説明
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
}

/// ローカル AI が認識した途中の文と訳
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LiveText {
    pub text: String,
    /// ふりがなと用語の付いた文
    #[serde(default)]
    pub tokens: Vec<Token>,
    #[serde(default)]
    pub translations: Translations,
}

/// 字幕を始められなかった・止まった理由。文は操作パネルが作る（サーバーは中身だけを返す）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "code",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum TranscriptionError {
    MissingKey,
    /// ギフトコードの期限が切れた（DESIGN.md 17.1）
    GiftExpired {
        #[ts(type = "number")]
        expires_at: i64,
    },
    /// おすそ分けした人がギフトコードを止めた
    GiftRevoked,
    /// ないギフトコード（おすそ分けをやめたなど）
    GiftUnknown,
    /// マイクの入力が途切れた
    InputEnded,
    /// マイクを開けない
    Microphone {
        #[serde(flatten)]
        error: crate::audio::MicError,
    },
    /// そのほかのエラー（通信や API のエラー。`detail` は英語の技術的な説明）
    Failed {
        detail: String,
    },
    /// 想定外のエラー（panic）
    Crashed {
        detail: String,
    },
}

impl TranscriptionError {
    pub fn from_error(e: &anyhow::Error) -> Self {
        match e.downcast_ref::<crate::audio::MicError>() {
            Some(error) => Self::Microphone {
                error: error.clone(),
            },
            None => Self::Failed {
                detail: format!("{e:#}"),
            },
        }
    }
}

impl ServerMsg {
    pub fn topic(&self) -> Topic {
        match self {
            Self::Snapshot { .. } | Self::SegmentUpsert { .. } => Topic::Subtitle,
            Self::GlossaryUpdated { .. } => Topic::Glossary,
            Self::RankUpdate { .. } => Topic::Rank,
            Self::TranscriptionStatus { .. } | Self::SettingsChanged | Self::LiveStatus { .. } => {
                Topic::Status
            }
            Self::Display { .. } => Topic::Display,
            Self::AudioLevel { .. } => Topic::Level,
            Self::Usage { .. } => Topic::Usage,
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
    /// 持ち主の何番目のスキルか（1〜4。4 がウルト）。「ヘイズのウルト」をこのスキルにする
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ability_slot: Option<u8>,
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
    /// `/assets/rank/{badge}.webp`。アイコンをまだ取れていなければ None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub image: Option<String>,
    /// 今のサブランクの中での進み具合（ゲージ）。エターナスと未ランクは None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub progress: Option<RankProgress>,
    /// エターナスのリーダーボードでの順位（地域を選んでいて、載っているときだけ）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub leaderboard: Option<LeaderboardRank>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LeaderboardRank {
    pub region: crate::settings::Region,
    pub rank: u32,
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
    pub anchor: crate::settings::Anchor,
}

/// 操作パネルに返す設定。API キーはどこから読んだかだけ（キーそのものは返さない）
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct SettingsView {
    /// アプリの版（操作パネルに出す）
    pub version: &'static str,
    pub settings: crate::settings::Settings,
    pub keys: crate::secrets::KeyStatus,
    /// 設定ファイルを読めない理由（OS のエラー）。そのときは変更をこの起動の間だけ当て、保存しない
    #[ts(optional)]
    pub unsaved: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microphone_error_is_flat_for_the_panel() {
        let e = TranscriptionError::Microphone {
            error: crate::audio::MicError::NotFound { name: "USB".into() },
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({ "code": "microphone", "problem": "notFound", "name": "USB" })
        );
    }
}
