//! 設定（DESIGN.md 5 章）。`{データディレクトリ}/settings.json` に保存し、操作パネルから変える。
//!
//! API キーはここには入れない（`secrets`。OS の資格情報ストアに保存する）。

use std::collections::BTreeMap;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::lang;
use crate::secrets::Provider;
use crate::types::LangCode;

/// `{データディレクトリ}` の中の設定ファイル
pub const FILE: &str = "settings.json";

/// 翻訳先の上限（多いほど出力が増えて遅くなる）
pub const MAX_TRANSLATE_TO: usize = 4;

/// オーバーレイのフォント名の上限（文字数）
const MAX_FONT_NAME: usize = 256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Settings {
    /// マイクを字幕にするか（操作パネルの開始・停止）
    pub transcription_enabled: bool,
    /// 字幕の工程ごとに、何で作るか（モードを選ぶと決まり、詳しく選ぶと変えられる）
    pub engines: Engines,
    /// 配信者が話す言語
    pub mic_language: LangCode,
    /// マイクのデバイス名。None なら OS の既定の入力デバイス
    #[ts(optional)]
    pub mic_device: Option<String>,
    /// 翻訳先（最大 4 言語。mic_language は除く）
    pub translate_to: Vec<LangCode>,
    /// ヒーロー・スキル・アイテムを判定して強調し、アイテム説明を出す。雑談の配信ならオフにする
    pub deadlock_mode: bool,
    /// 字幕オーバーレイの見せ方。変えると表示中のオーバーレイにそのまま届く
    pub subtitle_display: SubtitleDisplay,
    /// 字幕の文字の見た目
    pub subtitle_style: SubtitleStyle,
    /// アイテム説明のオーバーレイ（字幕でアイテム名が出たら説明のカードを出す）
    pub item_cards: ItemCards,
    /// ランク名の言語。None なら話す言語に合わせる。
    /// 既定は None のまま（前の版の設定ファイルを変えない）で、新しく使い始める人は英語（`load`）
    #[ts(optional)]
    pub rank_language: Option<LangCode>,
    /// ランクのオーバーレイに出すもの
    pub rank_content: RankContent,
    /// ランクをソースの箱のどこに寄せるか
    pub rank_anchor: Anchor,
    /// エターナスの順位を調べるリーダーボードの地域。None なら調べない
    #[ts(optional)]
    pub rank_region: Option<Region>,
    /// ランクの文字の見た目
    pub rank_style: RankStyle,
    /// OBS のブラウザソースで字幕オーバーレイが開かれたことがある（はじめかたのチェックに使う）
    pub obs_seen: bool,
    /// はじめかたを閉じた、または全部終えた
    pub onboarding_closed: bool,
    /// ウィンドウを閉じても動き続けることを知らせた
    pub tray_notice_seen: bool,
    /// 無料で使っている人への「おすすめにする」の提案を閉じた
    pub hybrid_hint_closed: bool,
    /// 最後に自分で組んだ工程の組み合わせ（モードにないもの）。操作パネルの「カスタム」
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_engines: Option<Engines>,
    /// 更新の内容（`CHANGELOG.md`）を最後に見た版（`0.6.0`。プレリリースの印は付けない）。
    /// None は、この項目ができる前（0.5 まで）から使っている人で、今の版の内容を見せる
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes_seen: Option<String>,
    // 以下の詳細項目は操作パネルで変えない（`ADVANCED`）
    pub models: Models,
    /// リアルタイムの音声認識の遅れ（`minimal` / `low` / `medium` / `high` / `xhigh`。
    /// 低いほど早く字幕が出て、高いほど正確）
    pub realtime_delay: String,
    pub segmentation: Segmentation,
    /// ランクを表示するアカウント（Steam の 32bit アカウント ID）。None ならランクを取りに行かない
    #[ts(optional)]
    pub rank_account: Option<u32>,
    /// ランクを取りに行く間隔（試合後すぐには反映されないので短くしない）
    #[ts(type = "number")]
    pub rank_poll_seconds: u64,
    /// 開発者のサーバー（おすそ分け。DESIGN.md 17.1）の根元。手元で試すときに `wrangler dev` に向ける
    pub api_base: String,
}

/// 無いフォントは CSS が次の候補に落とすので、確かめるのは名前の形だけ。
/// 引用符と `\` は `overlay-fonts.ts` がエスケープする
pub(crate) fn valid_font_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= MAX_FONT_NAME && !name.chars().any(char::is_control)
}

/// 既定値から変えたときだけ保存する項目（保存してしまうと、アプリの既定値を直しても古い値が残る）
const ADVANCED: [&str; 5] = [
    "models",
    "realtimeDelay",
    "segmentation",
    "rankPollSeconds",
    "apiBase",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Models {
    /// リアルタイムの音声認識（OpenAI）
    pub realtime: String,
    /// 確定字幕の音声認識（OpenAI。Grok を選ぶと `Settings::models_in_use` が替える）
    pub batch: String,
    /// 翻訳（OpenAI）。質を優先して選んだ（DESIGN.md 9.3）
    pub translate: String,
    /// 辞書の漢字名の読み（OpenAI）。名前ごとにキャッシュする
    pub readings: String,
    /// translate / readings の推論の深さ。推論しないモデルにするときは None
    #[ts(optional)]
    pub reasoning_effort: Option<String>,
}

/// 字幕オーバーレイの見せ方（DESIGN.md 11 章）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct SubtitleDisplay {
    /// 話した言葉を出す。オフなら翻訳だけを、届いてから出す
    pub show_source: bool,
    /// 話した言葉にふりがなを付ける
    pub furigana: bool,
    /// 字幕の揃え。オーバーレイは字幕の箱で、置く場所は OBS で決める
    pub align: Align,
    /// 話した言葉の文字の大きさ（px）
    pub font_size: u32,
    /// 訳文の文字の大きさ（px）
    pub translation_font_size: u32,
    /// 長い文に足す秒数（1 文字あたり）
    pub per_char_seconds: f32,
    /// 翻訳が届いてから、次の字幕に押し出すまでの秒数（DESIGN.md 11 章）
    pub min_read_seconds: f32,
    /// 新しい字幕を待たせる最長の秒数。0 なら待たせず、すぐ入れ替える
    pub max_wait_seconds: f32,
    /// 同時に出す字幕の数
    pub max_lines: u32,
    /// 確定してから消すまでの秒数（長い文はそのぶん長く出す）
    pub hold_seconds: f32,
}

impl Default for SubtitleDisplay {
    fn default() -> Self {
        Self {
            show_source: true,
            furigana: true,
            align: Align::Center,
            font_size: 40,
            translation_font_size: 26,
            per_char_seconds: 0.15,
            min_read_seconds: 3.0,
            max_wait_seconds: 6.0,
            max_lines: 3,
            hold_seconds: 6.0,
        }
    }
}

impl SubtitleDisplay {
    /// 0 は制限なし
    pub const MAX_LINES: std::ops::RangeInclusive<u32> = 0..=5;
    pub const HOLD_SECONDS: std::ops::RangeInclusive<f32> = 2.0..=30.0;
    pub const FONT_SIZE: std::ops::RangeInclusive<u32> = 12..=96;
    pub const PER_CHAR_SECONDS: std::ops::RangeInclusive<f32> = 0.0..=1.0;
    pub const MIN_READ_SECONDS: std::ops::RangeInclusive<f32> = 1.0..=30.0;
    pub const MAX_WAIT_SECONDS: std::ops::RangeInclusive<f32> = 0.0..=30.0;

    fn validate(&self) -> Result<(), String> {
        if !Self::MAX_LINES.contains(&self.max_lines)
            || !Self::HOLD_SECONDS.contains(&self.hold_seconds)
            || !Self::FONT_SIZE.contains(&self.font_size)
            || !Self::FONT_SIZE.contains(&self.translation_font_size)
            || !Self::PER_CHAR_SECONDS.contains(&self.per_char_seconds)
            || !Self::MIN_READ_SECONDS.contains(&self.min_read_seconds)
            || !Self::MAX_WAIT_SECONDS.contains(&self.max_wait_seconds)
        {
            return Err("subtitleDisplay is out of range".into());
        }
        Ok(())
    }
}

/// アイテム説明のオーバーレイ（DESIGN.md 12.5）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ItemCards {
    /// 1 枚のカードを出しておく秒数
    pub hold_seconds: f32,
    /// 説明の言語。None なら話す言語
    #[ts(optional)]
    pub language: Option<LangCode>,
    /// カードをソースの箱のどこに寄せるか
    pub anchor: Anchor,
}

impl Default for ItemCards {
    fn default() -> Self {
        Self {
            hold_seconds: 6.0,
            language: None,
            anchor: Anchor::BottomRight,
        }
    }
}

impl ItemCards {
    pub const HOLD_SECONDS: std::ops::RangeInclusive<f32> = 3.0..=30.0;

    fn validate(&self) -> Result<(), String> {
        if !Self::HOLD_SECONDS.contains(&self.hold_seconds) {
            return Err(format!("holdSeconds must be in {:?}", Self::HOLD_SECONDS));
        }
        match self.language.as_deref() {
            Some(code) if lang::find(code).is_none() => Err(format!("unknown language: {code}")),
            _ => Ok(()),
        }
    }

    fn apply(&self, patch: ItemCardsPatch) -> Result<Self, String> {
        let next = Self {
            hold_seconds: patch.hold_seconds.unwrap_or(self.hold_seconds),
            language: match patch.language {
                Some(code) => Some(code).filter(|c| !c.is_empty()),
                None => self.language.clone(),
            },
            anchor: patch.anchor.unwrap_or(self.anchor),
        };
        next.validate()?;
        Ok(next)
    }
}

/// アイテム説明の設定の変更。省いた項目は変えない
#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ItemCardsPatch {
    #[ts(optional)]
    pub hold_seconds: Option<f32>,
    /// 空文字列なら話す言語
    #[ts(optional)]
    pub language: Option<LangCode>,
    #[ts(optional)]
    pub anchor: Option<Anchor>,
}

/// 字幕の見せ方の変更。省いた項目は変えない（続けて変えたとき、前の変更を古い値で戻さない）
#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SubtitleDisplayPatch {
    #[ts(optional)]
    pub show_source: Option<bool>,
    #[ts(optional)]
    pub furigana: Option<bool>,
    #[ts(optional)]
    pub align: Option<Align>,
    #[ts(optional)]
    pub font_size: Option<u32>,
    #[ts(optional)]
    pub translation_font_size: Option<u32>,
    #[ts(optional)]
    pub per_char_seconds: Option<f32>,
    #[ts(optional)]
    pub min_read_seconds: Option<f32>,
    #[ts(optional)]
    pub max_wait_seconds: Option<f32>,
    #[ts(optional)]
    pub max_lines: Option<u32>,
    #[ts(optional)]
    pub hold_seconds: Option<f32>,
}

impl SubtitleDisplay {
    fn apply(&self, patch: SubtitleDisplayPatch) -> Result<Self, String> {
        let next = Self {
            show_source: patch.show_source.unwrap_or(self.show_source),
            furigana: patch.furigana.unwrap_or(self.furigana),
            align: patch.align.unwrap_or(self.align),
            font_size: patch.font_size.unwrap_or(self.font_size),
            translation_font_size: patch
                .translation_font_size
                .unwrap_or(self.translation_font_size),
            per_char_seconds: patch.per_char_seconds.unwrap_or(self.per_char_seconds),
            min_read_seconds: patch.min_read_seconds.unwrap_or(self.min_read_seconds),
            max_wait_seconds: patch.max_wait_seconds.unwrap_or(self.max_wait_seconds),
            max_lines: patch.max_lines.unwrap_or(self.max_lines),
            hold_seconds: patch.hold_seconds.unwrap_or(self.hold_seconds),
        };
        next.validate()?;
        Ok(next)
    }
}

/// 字幕とランクで共通の文字の見た目（DESIGN.md 11 章）。None は標準で、オーバーレイに値を渡さず、
/// CSS の既定の見た目と OBS のカスタム CSS で上書きした変数をそのまま使う
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct TextLook {
    /// 通常の文字の色。ヒーロー名などの色分けは変えない
    pub color: Option<String>,
    pub outline: Outline,
    pub shadow: Shadow,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Outline {
    /// 文字の大きさとの比（em）
    pub width: Option<f64>,
    pub color: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Shadow {
    /// None なら標準（字幕は帯のないときだけ、ランクはいつも付ける）
    pub enabled: Option<bool>,
    /// ぼかしの半径（em）
    pub blur: Option<f64>,
    pub color: Option<String>,
}

/// 字幕の行の後ろに敷く半透明の帯。ランクには敷かない（アイコンの透明な縁がティアごとに違い、余白がそろわない）
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Background {
    pub enabled: bool,
    pub color: Option<String>,
    /// 濃さ（0〜1）
    pub opacity: Option<f64>,
}

/// フォントに付く項目。字幕は言語ごと、ランクは 1 つ
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct FontStyle {
    /// ファミリー名
    pub font: Option<String>,
    pub weight: Option<u16>,
    /// 字間（em）
    pub letter_spacing: Option<f64>,
}

/// 字幕の文字の見た目。言語ごとに色なども変えられるようにするなら `FontStyle` の隣に足す
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct SubtitleStyle {
    #[serde(flatten)]
    pub look: TextLook,
    pub background: Background,
    pub line_height: Option<f64>,
    /// キーがない言語は標準
    pub languages: BTreeMap<LangCode, FontStyle>,
}

/// ランクの文字の見た目
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct RankStyle {
    #[serde(flatten)]
    pub look: TextLook,
    #[serde(flatten)]
    pub font: FontStyle,
}

const OUTLINE_WIDTH: std::ops::RangeInclusive<f64> = 0.0..=0.5;
const SHADOW_BLUR: std::ops::RangeInclusive<f64> = 0.0..=1.0;
const OPACITY: std::ops::RangeInclusive<f64> = 0.0..=1.0;
const LINE_HEIGHT: std::ops::RangeInclusive<f64> = 1.0..=3.0;
/// CSS の `font-weight` と同じ範囲。フォントの面は 950 などもある
const WEIGHT: std::ops::RangeInclusive<u16> = 1..=1000;
const LETTER_SPACING: std::ops::RangeInclusive<f64> = -0.1..=0.5;
/// ランクの既定の箱（幅 400）に、長い名前が収まる字間
const RANK_LETTER_SPACING: std::ops::RangeInclusive<f64> = -0.1..=0.1;

fn valid_color(color: &Option<String>) -> bool {
    color.as_deref().is_none_or(|c| {
        c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit())
    })
}

fn within<T: PartialOrd>(value: Option<T>, range: &std::ops::RangeInclusive<T>) -> bool {
    value.is_none_or(|v| range.contains(&v))
}

impl TextLook {
    fn validate(&self) -> Result<(), String> {
        let valid = valid_color(&self.color)
            && valid_color(&self.outline.color)
            && valid_color(&self.shadow.color)
            && within(self.outline.width, &OUTLINE_WIDTH)
            && within(self.shadow.blur, &SHADOW_BLUR);
        valid
            .then_some(())
            .ok_or("text style is out of range".into())
    }
}

impl FontStyle {
    fn validate(&self) -> Result<(), String> {
        if let Some(name) = self.font.as_deref().filter(|f| !valid_font_name(f)) {
            return Err(format!("invalid font name: {name}"));
        }
        let valid = within(self.weight, &WEIGHT) && within(self.letter_spacing, &LETTER_SPACING);
        valid
            .then_some(())
            .ok_or("font style is out of range".into())
    }
}

impl SubtitleStyle {
    fn validate(&self) -> Result<(), String> {
        self.look.validate()?;
        if !valid_color(&self.background.color) || !within(self.background.opacity, &OPACITY) {
            return Err("background is out of range".into());
        }
        if !within(self.line_height, &LINE_HEIGHT) {
            return Err("lineHeight is out of range".into());
        }
        for (language, font) in &self.languages {
            if lang::find(language).is_none() {
                return Err(format!("unknown language: {language}"));
            }
            font.validate()?;
        }
        Ok(())
    }

    fn apply(&self, patch: &Value) -> Result<Self, String> {
        let mut next: Self = merged(self, patch)?;
        next.languages
            .retain(|_, font| *font != FontStyle::default());
        next.validate()?;
        Ok(next)
    }
}

impl RankStyle {
    fn validate(&self) -> Result<(), String> {
        self.look.validate()?;
        self.font.validate()?;
        within(self.font.letter_spacing, &RANK_LETTER_SPACING)
            .then_some(())
            .ok_or("letterSpacing is out of range".into())
    }

    fn apply(&self, patch: &Value) -> Result<Self, String> {
        let next: Self = merged(self, patch)?;
        next.validate()?;
        Ok(next)
    }
}

/// JSON Merge Patch（RFC 7386）。書いた項目だけを変え、null は標準に戻す
fn merge(target: &mut Value, patch: &Value) {
    let Value::Object(fields) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(serde_json::Map::new());
    }
    let target = target.as_object_mut().expect("is an object");
    for (key, value) in fields {
        if value.is_null() {
            target.remove(key);
        } else {
            merge(target.entry(key).or_insert(Value::Null), value);
        }
    }
}

fn merged<T: Serialize + DeserializeOwned>(current: &T, patch: &Value) -> Result<T, String> {
    let mut json = serde_json::to_value(current).map_err(|e| e.to_string())?;
    merge(&mut json, patch);
    serde_json::from_value(json).map_err(|e| e.to_string())
}

/// ランクのオーバーレイに出すもの
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum RankContent {
    /// アイコン・名前・ゲージ（ゲージのないエターナスは順位かポイント）
    Gauge,
    /// アイコンと名前
    Name,
    /// アイコンだけ
    Icon,
}

/// オーバーレイの中身をソースの箱のどこに寄せるか（3×3 のどれか）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

/// deadlock-api のリーダーボードの地域（値は API のパスのまま）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Region {
    Europe,
    Asia,
    NAmerica,
    SAmerica,
    Oceania,
}

impl Region {
    const ALL: [Region; 5] = [
        Region::Europe,
        Region::Asia,
        Region::NAmerica,
        Region::SAmerica,
        Region::Oceania,
    ];

    /// API のパス（`/v1/leaderboard/{region}`）と設定での名前
    pub fn as_api(self) -> &'static str {
        match self {
            Region::Europe => "Europe",
            Region::Asia => "Asia",
            Region::NAmerica => "NAmerica",
            Region::SAmerica => "SAmerica",
            Region::Oceania => "Oceania",
        }
    }

    fn from_api(name: &str) -> Option<Region> {
        Self::ALL.into_iter().find(|r| r.as_api() == name)
    }
}

/// 字幕の工程ごとに、何で作るか。無料（Chrome と PC の中）か有料（OpenAI・xAI）か（DESIGN.md 9.6）。
/// 今の仕組みで作れない組み合わせは `sanitized` が直す
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase", from = "SavedEngines")]
#[ts(export)]
pub struct Engines {
    /// 途中の字幕
    pub live: LiveSource,
    /// 確定の文字起こし
    pub transcription: FinalSource,
    /// 用語の判定。翻訳を LLM にしたら、LLM が翻訳と同時に判定するので使わない（DESIGN.md 9.7）
    pub terms: TermsEngine,
    /// 翻訳。LLM なら用語も同時に判定する（訳文の名前を用語から作る。DESIGN.md 9.1）
    pub translation: TranslationEngine,
    /// ふりがな
    pub furigana: RubySource,
}

/// 設定ファイルの中の `Engines`。消した選択肢を、今の選択肢に読み替える
#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SavedEngines {
    live: LiveSource,
    transcription: FinalSource,
    terms: TermsEngine,
    translation: SavedTranslation,
    furigana: RubySource,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SavedTranslation {
    Engine(TranslationEngine),
    Removed(RemovedTranslation),
}

/// 消した翻訳の選択肢
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum RemovedTranslation {
    /// 0.6.0-rc.6 までの「GPT-6 Luna（用語も）」。ベータの用語と翻訳が、精度も速さも料金も上なので消した（DESIGN.md 9.6）
    Gpt6Luna,
}

impl Default for SavedEngines {
    fn default() -> Self {
        let Engines {
            live,
            transcription,
            terms,
            translation,
            furigana,
        } = Engines::default();
        Self {
            live,
            transcription,
            terms,
            translation: SavedTranslation::Engine(translation),
            furigana,
        }
    }
}

impl From<SavedEngines> for Engines {
    fn from(saved: SavedEngines) -> Self {
        let SavedEngines {
            live,
            transcription,
            terms,
            translation,
            furigana,
        } = saved;
        let (terms, translation) = match translation {
            SavedTranslation::Engine(translation) => (terms, translation),
            SavedTranslation::Removed(RemovedTranslation::Gpt6Luna) => {
                (Self::BETA.terms, Self::BETA.translation)
            }
        };
        Self {
            live,
            transcription,
            terms,
            translation,
            furigana,
        }
    }
}

impl Engines {
    /// Chrome のモード。すべて無料で、新しく使い始める人の既定
    pub const CHROME: Self = Self {
        live: LiveSource::Chrome,
        transcription: FinalSource::Chrome,
        terms: TermsEngine::Dictionary,
        translation: TranslationEngine::ChromeTranslator,
        furigana: RubySource::Dictionary,
    };
    /// ベータのモード。ベータ期間中は無料（開発者のサーバーが、開発者のキーで中継する。DESIGN.md 17.2）。
    /// いちばん安い組み合わせ: 途中の字幕は Chrome、確定は Qwen、用語は Perplexity Decider、翻訳は Hy-MT2、
    /// ふりがなは辞書の候補から Perplexity Decider が選ぶ
    pub const BETA: Self = Self {
        live: LiveSource::Chrome,
        transcription: FinalSource::QwenAudio,
        terms: TermsEngine::PplxDecider,
        translation: TranslationEngine::HyMt2,
        furigana: RubySource::Decider,
    };
    /// 操作パネルに出すモード（おすすめの組み合わせ）。ほかの組み合わせは「カスタム」（`Settings::custom_engines`）。
    /// `STANDARD` は「最高精度」（OpenAI の鍵が要る）で、ほかは鍵なしで始められる
    pub const MODES: [Self; 3] = [Self::BETA, Self::CHROME, Self::STANDARD];
    /// 最高精度のモード（0.5 までの「スタンダード」）。途中の字幕は Chrome、確定は OpenAI
    pub const STANDARD: Self = Self {
        live: LiveSource::Chrome,
        transcription: FinalSource::GptTranscribe,
        terms: TermsEngine::Dictionary,
        translation: TranslationEngine::Gpt61Terra,
        furigana: RubySource::Llm,
    };
    /// OpenAI のモード。すべて OpenAI で、Chrome を使わない
    pub const OPENAI: Self = Self {
        live: LiveSource::GptLiveTranscribe,
        ..Self::STANDARD
    };

    /// 有料の工程があり、鍵が要る。表示の設定（翻訳先、ふりがな）では変わらないので、
    /// 表示を切り替えても字幕の処理を始め直さない
    pub fn paid(&self) -> bool {
        !self.providers().is_empty()
    }

    /// 工程が使う会社（鍵が要る）。`Provider::ALL` の順。どの選択肢がどの会社を使うかは、工程ごとの `provider`
    pub fn providers(&self) -> Vec<Provider> {
        let used = [
            self.live.provider(),
            self.transcription.provider(),
            self.judge().map(|judge| judge.provider),
            self.translation.provider(),
            self.furigana.provider(),
        ];
        let all = Provider::ALL.into_iter();
        all.filter(|provider| used.contains(&Some(*provider)))
            .collect()
    }

    /// その会社を使う工程があり、鍵が要る
    pub fn uses(&self, provider: Provider) -> bool {
        self.providers().contains(&provider)
    }

    /// PC の中の文字起こし（speech.cpp）を立てる工程がある
    pub fn uses_local(&self) -> bool {
        self.live == LiveSource::HaasJa || self.transcription == FinalSource::HaasJa
    }

    /// 用語の候補を確かめさせる相手（翻訳が LLM なら、LLM が同時に判定するので使わない）
    pub fn judge(&self) -> Option<Model> {
        self.terms
            .judge()
            .filter(|_| self.translation.source() != TranslationSource::Llm)
    }

    /// 用語を判定モデルで確かめる
    pub fn decides_terms(&self) -> bool {
        self.judge().is_some()
    }

    /// 確定の文を Chrome が作るなら、発話の区切りも Chrome が決めるので、途中の字幕も Chrome。
    /// Chrome の翻訳はページの中で動くので、途中の字幕が Chrome のときだけ使える
    pub fn sanitized(mut self) -> Self {
        if self.transcription == FinalSource::Chrome {
            self.live = LiveSource::Chrome;
        }
        if self.live != LiveSource::Chrome
            && self.translation == TranslationEngine::ChromeTranslator
        {
            self.translation = TranslationEngine::Gpt61Terra;
        }
        // Gemini Nano は Chrome のページの中にある
        if self.live != LiveSource::Chrome && self.terms == TermsEngine::GeminiNano {
            self.terms = TermsEngine::Dictionary;
        }
        self
    }
}

/// 画面と LP が使う一覧（`frontend/src/generated/catalog.ts`）。会社の名前とギフトコードで使えるか、
/// 工程の選択肢ごとの会社。同じことを TypeScript に手で書かない（`cargo test` が書き出す）
pub fn catalog() -> String {
    fn step<T: Serialize + Copy>(all: &[T], provider: impl Fn(T) -> Option<Provider>) -> Value {
        let options = all.iter().map(|option| {
            let name = serde_json::to_value(option).expect("a name");
            let name = name.as_str().expect("a name").to_owned();
            (name, serde_json::json!(provider(*option)))
        });
        Value::Object(options.collect())
    }
    let providers: Vec<Value> = Provider::ALL
        .into_iter()
        .map(|provider| {
            let info = provider.info();
            serde_json::json!({ "id": info.id, "name": info.name, "relay": info.relay })
        })
        .collect();
    let engines = serde_json::json!({
        "live": step(&LiveSource::ALL, LiveSource::provider),
        "transcription": step(&FinalSource::ALL, FinalSource::provider),
        "terms": step(&TermsEngine::ALL, |terms| terms.judge().map(|judge| judge.provider)),
        "translation": step(&TranslationEngine::ALL, TranslationEngine::provider),
        "furigana": step(&RubySource::ALL, RubySource::provider),
    });
    let json = |value: &Value| serde_json::to_string_pretty(value).expect("JSON");
    format!(
        "// crates/core/src/settings.rs の `catalog` が作る（`cargo test`）。手で書き換えない\n\n\
         /** 会社。鍵が要る順 */\n\
         export const PROVIDERS = {} as const;\n\n\
         /** 工程の選択肢ごとの、使う会社（無料なら null） */\n\
         export const ENGINE_PROVIDERS = {} as const;\n",
        json(&Value::Array(providers)),
        json(&engines),
    )
}

impl Default for Engines {
    fn default() -> Self {
        Self::CHROME
    }
}

/// 途中の字幕を何で作るか
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LiveSource {
    Chrome,
    GptLiveTranscribe,
    /// QwenCloud の Qwen-Audio-3.1-ASR-Flash-Streaming。GPT-Live-Transcribe の約 20 分の 1 の料金（DESIGN.md 3.4）
    QwenAudioStreaming,
    /// PC の中で動く Haas-ja（speech.cpp）。話している間、同じ発話を何度も認識し直して出す。無料。日本語だけ（DESIGN.md 9.8）
    HaasJa,
    /// 話し終えてから出す。途中の字幕の料金がかからない
    Off,
}

impl LiveSource {
    pub const ALL: [Self; 5] = [
        Self::Chrome,
        Self::GptLiveTranscribe,
        Self::QwenAudioStreaming,
        Self::HaasJa,
        Self::Off,
    ];

    pub fn provider(self) -> Option<Provider> {
        match self {
            Self::Chrome | Self::HaasJa | Self::Off => None,
            Self::GptLiveTranscribe => Some(Provider::OpenAi),
            Self::QwenAudioStreaming => Some(Provider::Qwen),
        }
    }
}

/// 確定の文字起こしを何で作るか
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FinalSource {
    Chrome,
    GptTranscribe,
    /// GPT-Transcribe より安い。精度は同じか少し下（DESIGN.md 3）
    GrokVoiceTranscribe,
    /// QwenCloud の Qwen-Audio-3.1-ASR-Flash。Grok の約 7 分の 1 の料金で、少し遅い（DESIGN.md 3.4）
    QwenAudio,
    /// OpenRouter の MAI-Transcribe 2。Grok と同じくらい（DESIGN.md 9.7）
    MaiTranscribe2,
    /// PC の中で動く Haas-ja（speech.cpp）。無料。日本語だけ（DESIGN.md 9.8）
    HaasJa,
}

impl FinalSource {
    pub const ALL: [Self; 6] = [
        Self::Chrome,
        Self::GptTranscribe,
        Self::GrokVoiceTranscribe,
        Self::QwenAudio,
        Self::MaiTranscribe2,
        Self::HaasJa,
    ];

    /// 文字起こしを頼む会社（Chrome と PC の中なら None）
    pub fn provider(self) -> Option<Provider> {
        match self {
            Self::Chrome | Self::HaasJa => None,
            Self::GptTranscribe => Some(Provider::OpenAi),
            Self::GrokVoiceTranscribe => Some(Provider::Xai),
            Self::QwenAudio => Some(Provider::Qwen),
            Self::MaiTranscribe2 => Some(Provider::OpenRouter),
        }
    }

    /// 文字起こしのモデル。GPT-Transcribe は詳細設定の `models.batch`
    fn model(self) -> Option<&'static str> {
        match self {
            Self::Chrome | Self::GptTranscribe => None,
            Self::GrokVoiceTranscribe => Some("grok-voice-transcribe-2.0"),
            Self::QwenAudio => Some("qwen-audio-3.1-asr-flash"),
            Self::MaiTranscribe2 => Some("microsoft/mai-transcribe-2"),
            Self::HaasJa => Some(crate::local::HAAS_JA),
        }
    }
}

/// 会社と、その会社でのモデルの名前
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Model {
    pub provider: Provider,
    pub name: &'static str,
}

/// 字幕の文字の出どころ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSource {
    Chrome,
    GptLiveTranscribe,
    GptTranscribe,
    GrokVoiceTranscribe,
    QwenAudio,
    QwenAudioStreaming,
    MaiTranscribe2,
    HaasJa,
}

/// 用語を何で判定するか（翻訳が LLM でないとき）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TermsEngine {
    /// 辞書の名前との一致（PC の中）
    Dictionary,
    /// PC の中で候補を出し（名前、別名、似た名前、「ウルト」と番号）、OpenAI の Decisions API に確かめさせる
    Decisions,
    /// 同じ候補を、ほかの判定モデルに確かめさせる（DESIGN.md 9.7 の実測）。TypeSafe AI の Jev はいちばん速い
    Jev,
    /// OpenRouter の Nace Drex Max。いちばんよく当たった
    DrexMax,
    /// OpenRouter の Microsoft Decision 1
    Decision1,
    /// OpenRouter の Perplexity Decider V1.2。いちばん安い
    PplxDecider,
    /// 同じ候補を、Chrome の中の Gemini Nano に確かめさせる。無料で、文字を PC から出さない。
    /// 会社の判定モデルほどは当たらない（DESIGN.md 9.7）。途中の字幕が Chrome のときだけ使える
    GeminiNano,
}

impl TermsEngine {
    pub const ALL: [Self; 7] = [
        Self::Dictionary,
        Self::GeminiNano,
        Self::Decisions,
        Self::Jev,
        Self::DrexMax,
        Self::Decision1,
        Self::PplxDecider,
    ];

    /// 候補を確かめさせるモデル（辞書の一致なら None）
    pub fn judge(self) -> Option<Model> {
        let (provider, name) = match self {
            Self::Dictionary | Self::GeminiNano => return None,
            Self::Decisions => (Provider::OpenAi, "gpt-6-luna"),
            Self::Jev => (Provider::TypeSafe, "jev-latest"),
            Self::DrexMax => (Provider::OpenRouter, "nace-ai/drex-max"),
            Self::Decision1 => (Provider::OpenRouter, "microsoft/microsoft-decision-1"),
            Self::PplxDecider => (Provider::OpenRouter, "perplexity/pplx-decider-v1.2"),
        };
        Some(Model { provider, name })
    }
}

/// 翻訳を何で作るか。LLM（Terra）は用語も同時に判定する
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TranslationEngine {
    ChromeTranslator,
    /// 翻訳専門のモデル。速くて安い（DESIGN.md 9.7）
    QwenMtFlash,
    /// OpenRouter の Hy-MT2-30B-A3B。Qwen-MT-Flash の半額で、少し遅い（DESIGN.md 9.7）
    HyMt2,
    Gpt61Terra,
}

impl TranslationEngine {
    pub const ALL: [Self; 4] = [
        Self::ChromeTranslator,
        Self::QwenMtFlash,
        Self::HyMt2,
        Self::Gpt61Terra,
    ];

    pub fn source(self) -> TranslationSource {
        match self {
            Self::ChromeTranslator => TranslationSource::ChromeTranslator,
            Self::QwenMtFlash | Self::HyMt2 => TranslationSource::Mt,
            Self::Gpt61Terra => TranslationSource::Llm,
        }
    }

    /// 翻訳を頼む会社（LLM は OpenAI）
    pub fn provider(self) -> Option<Provider> {
        match self.source() {
            TranslationSource::ChromeTranslator => None,
            TranslationSource::Mt => self.mt().map(|mt| mt.provider),
            TranslationSource::Llm => Some(Provider::OpenAi),
        }
    }

    /// 翻訳専門のモデル
    pub fn mt(self) -> Option<Model> {
        let (provider, name) = match self {
            Self::QwenMtFlash => (Provider::Qwen, "qwen-mt-flash"),
            Self::HyMt2 => (Provider::OpenRouter, "tencent/hy-mt2-30b-a3b"),
            Self::ChromeTranslator | Self::Gpt61Terra => return None,
        };
        Some(Model { provider, name })
    }
}

/// 翻訳の出どころ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranslationSource {
    ChromeTranslator,
    /// 翻訳専門のモデル（Qwen-MT-Flash、Hy-MT2）
    Mt,
    Llm,
}

/// ふりがなの出どころ
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RubySource {
    /// 辞書（PC の中）。0.6.0-rc.8 までは、そのころの辞書の名前の `lindera` で保存していた
    #[serde(alias = "lindera")]
    Dictionary,
    /// 辞書の読みを、GPT-6.1 Terra（`models.readings`）に文に合わせて直させる
    Llm,
    /// 同じことを GPT-6 Luna にさせる。Terra の約 7 分の 1 の料金で、直せる読みは少し減る（DESIGN.md 6.4）
    Luna,
    /// 読みが分かれるところを、辞書の候補から判定モデル（OpenRouter の Perplexity Decider）に選ばせる。
    /// 速くて安く、ベータの中継で使える。直せる読みは Luna より少し減る（DESIGN.md 6.4）
    Decider,
}

impl RubySource {
    pub const ALL: [Self; 4] = [Self::Dictionary, Self::Decider, Self::Luna, Self::Llm];

    /// 辞書の読みを LLM に直させる
    pub fn llm(self) -> bool {
        matches!(self, Self::Llm | Self::Luna)
    }

    /// 辞書の候補から選ばせる判定モデル
    pub fn judge(self) -> Option<Model> {
        (self == Self::Decider).then_some(Model {
            provider: Provider::OpenRouter,
            name: "perplexity/pplx-decider-v1.2",
        })
    }

    pub fn provider(self) -> Option<Provider> {
        match self.judge() {
            Some(judge) => Some(judge.provider),
            None => self.llm().then_some(Provider::OpenAi),
        }
    }
}

/// 用語の強調の出どころ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmphasisSource {
    /// 辞書の名前との一致
    Terms,
    /// 候補を Decisions API で確かめる。確かめられなければ辞書の一致を使う
    Decisions,
    /// 候補を Chrome の中の Gemini Nano で確かめる。確かめられなければ辞書の一致を使う
    Chrome,
    Llm,
}

/// 字幕の途中か確定で、それぞれを何で作るか。None は作らない
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phase {
    pub text: Option<TextSource>,
    pub translation: Option<TranslationSource>,
    pub ruby: Option<RubySource>,
    pub emphasis: Option<EmphasisSource>,
}

/// 工程ごとの選択と設定から決まる、字幕の作り方。README の「しくみ」の表を写したもの。
/// 呼び出し側は工程ごとの選択ではなくこれを見る（DESIGN.md 9.6）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub partial: Phase,
    pub finals: Phase,
    /// 翻訳だけを見せる
    pub translation_only: bool,
}

impl Phase {
    const NONE: Self = Self {
        text: None,
        translation: None,
        ruby: None,
        emphasis: None,
    };

    /// PC の中で読みか用語を付けるか
    pub fn marks(&self) -> bool {
        self.ruby.is_some() || self.local_terms()
    }

    /// PC の中で辞書の一致を付けるか（Decisions で確かめるときも、確かめられなかったときのために付ける）
    pub fn local_terms(&self) -> bool {
        matches!(
            self.emphasis,
            Some(EmphasisSource::Terms | EmphasisSource::Decisions | EmphasisSource::Chrome)
        )
    }
}

impl Plan {
    /// Chrome を起動するか
    pub fn uses_chrome(&self) -> bool {
        self.partial.text == Some(TextSource::Chrome)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// 字幕の区切り（DESIGN.md 3.1）
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Segmentation {
    /// この長さの無音で Segment を閉じる
    pub min_silence_ms: u32,
    /// この長さを超えたら、次の短い無音で閉じる
    pub max_segment_ms: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            transcription_enabled: false,
            engines: Engines::default(),
            mic_language: "ja".into(),
            mic_device: None,
            translate_to: vec!["en".into()],
            deadlock_mode: true,
            subtitle_display: SubtitleDisplay::default(),
            subtitle_style: SubtitleStyle::default(),
            item_cards: ItemCards::default(),
            rank_language: None,
            rank_content: RankContent::Gauge,
            rank_anchor: Anchor::TopLeft,
            obs_seen: false,
            onboarding_closed: false,
            hybrid_hint_closed: false,
            custom_engines: None,
            notes_seen: None,
            tray_notice_seen: false,
            rank_region: None,
            rank_style: RankStyle::default(),
            models: Models::default(),
            realtime_delay: "low".into(),
            segmentation: Segmentation::default(),
            rank_account: None,
            rank_poll_seconds: 180,
            api_base: "https://api.jimaku.deadlock.jp/v1".into(),
        }
    }
}

impl Default for Models {
    fn default() -> Self {
        Self {
            realtime: "gpt-live-transcribe".into(),
            batch: "gpt-transcribe".into(),
            translate: "gpt-6.1-terra".into(),
            readings: "gpt-6.1-terra".into(),
            reasoning_effort: Some("none".into()),
        }
    }
}

impl Segmentation {
    /// 操作パネルで選べる無音の長さ
    pub const MIN_SILENCE_MS: std::ops::RangeInclusive<u32> = 200..=2000;
}

impl Default for Segmentation {
    fn default() -> Self {
        Self {
            min_silence_ms: 750,
            max_segment_ms: 8000,
        }
    }
}

impl Settings {
    /// アイテム説明に使う言語
    pub fn item_language(&self) -> LangCode {
        self.item_cards
            .language
            .clone()
            .unwrap_or_else(|| self.mic_language.clone())
    }

    /// ランク名に使う言語。選んだ言語の名前を取っていなければ（翻訳先から外したなど）話す言語
    pub fn rank_language(&self) -> LangCode {
        self.rank_language
            .clone()
            .filter(|l| self.glossary_languages().contains(l))
            .unwrap_or_else(|| self.mic_language.clone())
    }

    /// 工程ごとの選択と設定から決まる、字幕の作り方
    pub fn plan(&self) -> Plan {
        use EmphasisSource as Emphasis;
        use RubySource as Ruby;
        use TextSource as Text;
        use TranslationSource as Translation;

        let engines = self.engines;
        let translates = !self.translate_to.is_empty();
        let translation_only = translates && !self.subtitle_display.show_source;
        // 翻訳だけなら見えないので、作らない。形態素解析の辞書は日本語だけ
        let ruby = self.subtitle_display.furigana && !translation_only;
        let ja = self.mic_language == "ja";
        let dictionary = (ruby && ja).then_some(Ruby::Dictionary);
        let terms = (self.deadlock_mode && ja).then_some(Emphasis::Terms);
        let partial = match engines.live {
            LiveSource::Chrome => Phase {
                text: Some(Text::Chrome),
                translation: translates.then_some(Translation::ChromeTranslator),
                ruby: dictionary,
                emphasis: terms,
            },
            // 翻訳だけなら途中の字幕は見えないので、料金をかけない
            // アプリが作る途中の字幕（会社か、PC の中）
            LiveSource::GptLiveTranscribe | LiveSource::QwenAudioStreaming | LiveSource::HaasJa
                if !translation_only =>
            {
                Phase {
                    text: Some(match engines.live {
                        LiveSource::QwenAudioStreaming => Text::QwenAudioStreaming,
                        LiveSource::HaasJa => Text::HaasJa,
                        _ => Text::GptLiveTranscribe,
                    }),
                    translation: None,
                    ruby: dictionary,
                    emphasis: terms,
                }
            }
            LiveSource::GptLiveTranscribe
            | LiveSource::QwenAudioStreaming
            | LiveSource::HaasJa
            | LiveSource::Off => Phase::NONE,
        };
        let finals = Phase {
            text: Some(match engines.transcription {
                FinalSource::Chrome => Text::Chrome,
                FinalSource::GptTranscribe => Text::GptTranscribe,
                FinalSource::GrokVoiceTranscribe => Text::GrokVoiceTranscribe,
                FinalSource::QwenAudio => Text::QwenAudio,
                FinalSource::MaiTranscribe2 => Text::MaiTranscribe2,
                FinalSource::HaasJa => Text::HaasJa,
            }),
            translation: translates.then_some(engines.translation.source()),
            ruby: match engines.furigana {
                Ruby::Dictionary => dictionary,
                // 直すもとの辞書の読みは日本語だけ
                Ruby::Llm | Ruby::Luna | Ruby::Decider => dictionary.map(|_| engines.furigana),
            },
            emphasis: match (engines.translation.source(), engines.terms) {
                (Translation::Llm, _) => self.deadlock_mode.then_some(Emphasis::Llm),
                (_, TermsEngine::Dictionary) => terms,
                // 候補の出し方（かなの似た名前）は日本語だけ
                (_, TermsEngine::GeminiNano) => terms.map(|_| Emphasis::Chrome),
                (_, _) => terms.map(|_| Emphasis::Decisions),
            },
        };
        Plan {
            partial,
            finals,
            translation_only,
        }
    }

    /// 使うモデル。確定の文字起こしと、翻訳と用語は工程の選択で決まる（GPT-Transcribe と Terra は
    /// 詳細設定の `models.batch` と `models.translate` のまま）
    pub fn models_in_use(&self) -> Models {
        let mut models = self.models.clone();
        if let Some(model) = self.engines.transcription.model() {
            models.batch = model.into();
        }
        match self.engines.live {
            LiveSource::QwenAudioStreaming => {
                models.realtime = "qwen-audio-3.1-asr-flash-streaming".into();
            }
            LiveSource::HaasJa => models.realtime = crate::local::HAAS_JA.into(),
            LiveSource::Chrome | LiveSource::GptLiveTranscribe | LiveSource::Off => {}
        }
        if self.engines.furigana == RubySource::Luna {
            models.readings = "gpt-6-luna".into();
        }
        models
    }

    /// 辞書に名前を載せる言語（DESIGN.md 8.2）
    pub fn glossary_languages(&self) -> Vec<LangCode> {
        lang::normalize(
            [self.mic_language.clone(), "en".into()]
                .into_iter()
                .chain(self.translate_to.iter().cloned()),
        )
    }

    /// 手で編集されたファイルなども、操作パネルと同じ規則に揃える（知らない言語は既定、翻訳先は最大 4）
    fn sanitized(mut self) -> Self {
        if lang::find(&self.mic_language).is_none() {
            self.mic_language = Settings::default().mic_language;
        }
        let mic = self.mic_language.clone();
        self.engines = self.engines.sanitized();
        self.translate_to = lang::normalize(self.translate_to)
            .into_iter()
            .filter(|t| *t != mic)
            .take(MAX_TRANSLATE_TO)
            .collect();
        // 音声認識は Mistral から OpenAI に替えた。前に保存した Voxtral のモデルは使えない
        let defaults = Models::default();
        for (model, default) in [
            (&mut self.models.realtime, defaults.realtime),
            (&mut self.models.batch, defaults.batch),
        ] {
            if model.starts_with("voxtral") {
                *model = default;
            }
        }
        self.subtitle_style
            .languages
            .retain(|_, font| *font != FontStyle::default());
        self
    }

    /// 変更と同じ範囲に収まっているか。読めない値が残ると、ほかの項目を変えるときの検証も通らない
    fn valid(&self) -> bool {
        self.subtitle_display.validate().is_ok()
            && self.subtitle_style.validate().is_ok()
            && self.rank_style.validate().is_ok()
            && self.item_cards.validate().is_ok()
    }

    /// 操作パネルからの変更を当てる。読めない値はエラーにして何も変えない
    pub fn apply(&self, patch: SettingsPatch) -> Result<Self, String> {
        let mut next = self.clone();
        if let Some(enabled) = patch.transcription_enabled {
            next.transcription_enabled = enabled;
        }
        if let Some(device) = patch.mic_device {
            next.mic_device = Some(device).filter(|d| !d.is_empty());
        }
        if let Some(code) = patch.mic_language {
            if lang::find(&code).is_none() {
                return Err(format!("unknown language: {code}"));
            }
            next.mic_language = code;
        }
        if let Some(targets) = patch.translate_to {
            next.translate_to = targets;
        }
        // 話す言語を変えたときも、翻訳先から同じ言語を外す
        let targets = lang::normalize(next.translate_to.iter().cloned());
        next.translate_to = targets
            .into_iter()
            .filter(|t| *t != next.mic_language)
            .collect();
        if next.translate_to.len() > MAX_TRANSLATE_TO {
            return Err(format!(
                "translateTo accepts up to {MAX_TRANSLATE_TO} languages"
            ));
        }
        if let Some(display) = patch.subtitle_display {
            next.subtitle_display = next.subtitle_display.apply(display)?;
        }
        if let Some(style) = patch.subtitle_style {
            next.subtitle_style = next.subtitle_style.apply(&style)?;
        }
        if let Some(style) = patch.rank_style {
            next.rank_style = next.rank_style.apply(&style)?;
        }
        if let Some(cards) = patch.item_cards {
            next.item_cards = next.item_cards.apply(cards)?;
        }
        if let Some(ms) = patch.min_silence_ms {
            if !Segmentation::MIN_SILENCE_MS.contains(&ms) {
                return Err(format!(
                    "minSilenceMs must be in {:?}",
                    Segmentation::MIN_SILENCE_MS
                ));
            }
            next.segmentation.min_silence_ms = ms;
        }
        if let Some(code) = patch.rank_language {
            next.rank_language = match code.as_str() {
                "" => None,
                _ if lang::find(&code).is_none() => {
                    return Err(format!("unknown language: {code}"));
                }
                _ => Some(code),
            };
        }
        if let Some(enabled) = patch.deadlock_mode {
            next.deadlock_mode = enabled;
        }
        if let Some(engines) = patch.engines {
            next.engines = engines.sanitized();
            // 自分で組んだ組み合わせは覚えておく（モードを試したあとに戻れる）
            if !Engines::MODES.contains(&next.engines) {
                next.custom_engines = Some(next.engines);
            }
        }
        if let Some(content) = patch.rank_content {
            next.rank_content = content;
        }
        if let Some(anchor) = patch.rank_anchor {
            next.rank_anchor = anchor;
        }
        next.obs_seen = patch.obs_seen.unwrap_or(next.obs_seen);
        next.onboarding_closed = patch.onboarding_closed.unwrap_or(next.onboarding_closed);
        next.hybrid_hint_closed = patch.hybrid_hint_closed.unwrap_or(next.hybrid_hint_closed);
        next.tray_notice_seen = patch.tray_notice_seen.unwrap_or(next.tray_notice_seen);
        next.notes_seen = patch.notes_seen.or(next.notes_seen);
        if let Some(region) = patch.rank_region {
            next.rank_region = match region.as_str() {
                "" => None,
                name => {
                    Some(Region::from_api(name).ok_or_else(|| format!("unknown region: {name}"))?)
                }
            };
        }
        if let Some(account) = patch.rank_account {
            next.rank_account = match account.trim() {
                "" => None,
                value => Some(
                    parse_account_id(value)
                        .ok_or_else(|| format!("invalid Steam account ID: {value}"))?,
                ),
            };
        }
        Ok(next)
    }
}

/// 操作パネルから変える項目。省いた項目は変えない
#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SettingsPatch {
    #[ts(optional)]
    pub transcription_enabled: Option<bool>,
    /// 空文字列なら OS の既定の入力デバイス
    #[ts(optional)]
    pub mic_device: Option<String>,
    #[ts(optional)]
    pub mic_language: Option<LangCode>,
    #[ts(optional)]
    pub translate_to: Option<Vec<LangCode>>,
    #[ts(optional)]
    pub deadlock_mode: Option<bool>,
    /// 今の仕組みで作れない組み合わせは直して保存する
    #[ts(optional)]
    pub engines: Option<Engines>,
    /// SteamID64 か 32bit のアカウント ID。空文字列ならランクを表示しない
    #[ts(optional)]
    pub rank_account: Option<String>,
    #[ts(optional)]
    pub subtitle_display: Option<SubtitleDisplayPatch>,
    /// JSON Merge Patch。書いた項目だけを変え、null は標準に戻す
    #[ts(
        optional,
        type = r#"import("../lib/text-style").MergePatch<import("./SubtitleStyle").SubtitleStyle>"#
    )]
    pub subtitle_style: Option<Value>,
    /// `subtitle_style` と同じ
    #[ts(
        optional,
        type = r#"import("../lib/text-style").MergePatch<import("./RankStyle").RankStyle>"#
    )]
    pub rank_style: Option<Value>,
    #[ts(optional)]
    pub item_cards: Option<ItemCardsPatch>,
    /// 何 ms 黙ったら区切るか
    #[ts(optional)]
    pub min_silence_ms: Option<u32>,
    /// 空文字列なら話す言語
    #[ts(optional)]
    pub rank_language: Option<LangCode>,
    #[ts(optional)]
    pub rank_content: Option<RankContent>,
    #[ts(optional)]
    pub rank_anchor: Option<Anchor>,
    #[ts(optional)]
    pub obs_seen: Option<bool>,
    #[ts(optional)]
    pub onboarding_closed: Option<bool>,
    #[ts(optional)]
    pub hybrid_hint_closed: Option<bool>,
    #[ts(optional)]
    pub tray_notice_seen: Option<bool>,
    #[ts(optional)]
    pub notes_seen: Option<String>,
    /// 地域（`Region`）。空文字列なら順位を調べない
    #[ts(optional)]
    pub rank_region: Option<String>,
}

/// 読めない項目だけを捨てて、ほかの項目は残す。項目を 1 つだけ書いた設定として、葉から順に確かめる
fn keep_readable(value: &mut Value, path: &mut Vec<String>) -> bool {
    if let Value::Object(fields) = value {
        fields.retain(|key, field| {
            path.push(key.clone());
            let readable = keep_readable(field, path);
            if !readable {
                tracing::warn!(
                    "ignoring unreadable setting {}; using the default",
                    path.join(".")
                );
            }
            path.pop();
            readable
        });
    }
    let only = path.iter().rev().fold(value.clone(), |inner, key| {
        Value::Object(serde_json::Map::from_iter([(key.clone(), inner)]))
    });
    serde_json::from_value::<Settings>(only).is_ok_and(|settings| settings.valid())
}

/// ファイルがなければ既定値。壊れていたら脇に退けて既定値から始める。
/// 読めない（ほかのアプリが掴んでいるなど）ときは Err: 既定値で上書き保存しないため
/// 手で書き換えて読めなくなった項目があっても、ほかの設定まで失わないよう、その項目だけ既定値にする。
/// JSON として読めなければ Err
fn parse(bytes: &[u8]) -> serde_json::Result<Settings> {
    let mut json: Value = serde_json::from_slice(bytes)?;
    from_modes(&mut json);
    keep_readable(&mut json, &mut Vec::new());
    serde_json::from_value(json)
}

/// 0.4 までは 3 つのモード（`speechEngine`）から選び、OpenAI のモードだけ途中の字幕を切れた（`liveSubtitles`）。
/// 工程ごとの選択に置き換える
fn from_modes(json: &mut Value) {
    let Value::Object(fields) = json else {
        return;
    };
    let engine = fields.remove("speechEngine");
    let live = fields.remove("liveSubtitles");
    if fields.contains_key("engines") {
        return;
    }
    let engines = match engine.as_ref().and_then(Value::as_str) {
        Some("hybrid") => Engines::STANDARD,
        // 0.3 の rc には `local` や `free` で保存した版がある
        Some("chrome" | "local" | "free") => Engines::CHROME,
        // 0.3 より前の設定ファイルにはない（Chrome のウィンドウを黙って開かない）
        _ if live == Some(Value::Bool(false)) => Engines {
            live: LiveSource::Off,
            ..Engines::OPENAI
        },
        _ => Engines::OPENAI,
    };
    fields.insert(
        "engines".into(),
        serde_json::to_value(engines).expect("engines"),
    );
}

/// 今の版の、プレリリースの印を除いたもの（`0.6.0-rc.11` なら `0.6.0`）。更新の内容は、この版ごとに書く
fn release() -> &'static str {
    let version = env!("CARGO_PKG_VERSION");
    version
        .split_once('-')
        .map_or(version, |(release, _)| release)
}

pub fn load(path: &Path) -> std::io::Result<Settings> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // 新しく入れた人には、更新の内容を見せない（はじめかたがある）
            return Ok(Settings {
                rank_language: Some("en".into()),
                notes_seen: Some(release().to_owned()),
                ..Settings::default()
            });
        }
        Err(e) => return Err(e),
    };
    match parse(&bytes) {
        Ok(settings) => Ok(settings.sanitized()),
        Err(e) => {
            let aside = path.with_extension("json.broken");
            tracing::warn!("settings are broken ({e}); moved to {}", aside.display());
            std::fs::rename(path, aside)?;
            Ok(Settings::default())
        }
    }
}

pub fn save(path: &Path, settings: &Settings) -> anyhow::Result<()> {
    std::fs::create_dir_all(path.parent().expect("has parent"))?;
    let tmp = path.with_extension("json.tmp");
    let defaults = serde_json::to_value(Settings::default())?;
    let mut json = serde_json::to_value(settings)?;
    let fields = json.as_object_mut().expect("Settings is an object");
    fields.retain(|key, value| !ADVANCED.contains(&key.as_str()) || defaults[key] != *value);
    std::fs::write(&tmp, serde_json::to_vec_pretty(&json)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// SteamID64（76561198…）でも 32bit のアカウント ID でも受け付ける
pub fn parse_account_id(s: &str) -> Option<u32> {
    const STEAM_ID64_BASE: u64 = 76_561_197_960_265_728;
    let n: u64 = s.trim().parse().ok()?;
    u32::try_from(n.checked_sub(STEAM_ID64_BASE).unwrap_or(n)).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn account_id_accepts_steam_id64_and_account_id() {
        assert_eq!(parse_account_id("76561198268652547"), Some(308_386_819));
        assert_eq!(parse_account_id("308386819"), Some(308_386_819));
        assert_eq!(parse_account_id("abc"), None);
    }

    #[test]
    fn patch_changes_only_given_fields() {
        let s = Settings::default();
        let next = s
            .apply(SettingsPatch {
                rank_account: Some("76561198268652547".into()),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(next.rank_account, Some(308_386_819));
        assert_eq!(next.translate_to, s.translate_to);
        let cleared = next
            .apply(SettingsPatch {
                rank_account: Some(String::new()),
                mic_device: Some(String::new()),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(cleared.rank_account, None);
        assert_eq!(cleared.mic_device, None);
    }

    #[test]
    fn closing_the_hybrid_hint_is_saved_and_other_patches_keep_it() {
        let closed = Settings::default()
            .apply(SettingsPatch {
                hybrid_hint_closed: Some(true),
                ..SettingsPatch::default()
            })
            .unwrap();
        let saved = serde_json::to_string(&closed).unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        let next = loaded.apply(SettingsPatch::default()).unwrap();
        assert!(next.hybrid_hint_closed);
        assert!(!Settings::default().hybrid_hint_closed);
    }

    #[test]
    fn a_combination_of_my_own_is_remembered_while_i_try_a_mode() {
        let engines = |engines| SettingsPatch {
            engines: Some(engines),
            ..SettingsPatch::default()
        };
        let mine = Engines {
            terms: TermsEngine::Jev,
            ..Engines::BETA
        };
        // モードを選んでも、カスタムにはならない
        let beta = Settings::default().apply(engines(Engines::BETA)).unwrap();
        assert_eq!(beta.custom_engines, None);
        // 自分で組んだ組み合わせは、モードを試したあとも残る（保存しても）
        let custom = beta.apply(engines(mine)).unwrap();
        let tried = custom.apply(engines(Engines::CHROME)).unwrap();
        assert_eq!(
            (tried.engines, tried.custom_engines),
            (Engines::CHROME, Some(mine))
        );
        let saved = serde_json::to_string(&tried).unwrap();
        let loaded: Settings = serde_json::from_str(&saved).unwrap();
        assert_eq!(loaded.custom_engines, Some(mine));
        // 最高精度のほかは、鍵なしで始められる組み合わせ（ベータのコードか、Chrome）
        assert_eq!(Engines::STANDARD.providers(), [Provider::OpenAi]);
        for mode in Engines::MODES
            .into_iter()
            .filter(|mode| *mode != Engines::STANDARD)
        {
            let needs_own_key = |p: &Provider| p.info().relay != Some(crate::secrets::Relay::Beta);
            assert!(!mode.providers().iter().any(needs_own_key), "{mode:?}");
        }
    }

    #[test]
    fn patch_validates_languages_and_account() {
        let s = Settings::default();
        let bad = |patch| s.apply(patch).is_err();
        assert!(bad(SettingsPatch {
            mic_language: Some("xx".into()),
            ..SettingsPatch::default()
        }));
        assert!(bad(SettingsPatch {
            rank_account: Some("not-an-id".into()),
            ..SettingsPatch::default()
        }));
        assert!(bad(SettingsPatch {
            translate_to: Some(["en", "ru", "ko", "de", "fr"].map(String::from).to_vec()),
            ..SettingsPatch::default()
        }));
    }

    #[test]
    fn release_notes_are_for_people_who_updated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // 新しく入れた人は、今の版まで見たことにする
        let fresh = load(&path).unwrap();
        assert_eq!(fresh.notes_seen.as_deref(), Some(release()));
        assert!(!release().contains('-'));
        // この項目ができる前から使っている人は、まだ見ていない
        std::fs::write(&path, "{}").unwrap();
        let updated = load(&path).unwrap();
        assert_eq!(updated.notes_seen, None);
        // 見たら覚える。ほかの変更では消えない
        let seen = SettingsPatch {
            notes_seen: Some("0.6.0".into()),
            ..Default::default()
        };
        let seen = updated.apply(seen).unwrap();
        let later = seen.apply(SettingsPatch::default()).unwrap();
        assert_eq!(later.notes_seen.as_deref(), Some("0.6.0"));
    }

    #[test]
    fn patch_validates_display_pause_and_rank_language() {
        let s = Settings::default();
        let next = s
            .apply(SettingsPatch {
                subtitle_display: Some(SubtitleDisplayPatch {
                    max_lines: Some(2),
                    ..Default::default()
                }),
                min_silence_ms: Some(700),
                rank_language: Some("en".into()),
                ..Default::default()
            })
            .unwrap();
        // 送った項目だけが変わる
        assert_eq!(
            next.subtitle_display,
            SubtitleDisplay {
                max_lines: 2,
                ..SubtitleDisplay::default()
            }
        );
        assert_eq!(next.segmentation.min_silence_ms, 700);
        assert_eq!(next.rank_language(), "en");

        let reset = next
            .apply(SettingsPatch {
                rank_language: Some(String::new()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(reset.rank_language(), "ja");

        // 名前を取っていない言語（翻訳先から外した）は話す言語に戻る
        let ru = next
            .apply(SettingsPatch {
                rank_language: Some("ru".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(ru.rank_language(), "ja");

        for patch in [
            SettingsPatch {
                min_silence_ms: Some(50),
                ..Default::default()
            },
            SettingsPatch {
                subtitle_display: Some(SubtitleDisplayPatch {
                    max_lines: Some(6),
                    ..Default::default()
                }),
                ..Default::default()
            },
            SettingsPatch {
                subtitle_display: Some(SubtitleDisplayPatch {
                    translation_font_size: Some(200),
                    ..Default::default()
                }),
                ..Default::default()
            },
            SettingsPatch {
                rank_language: Some("xx".into()),
                ..Default::default()
            },
        ] {
            assert!(s.apply(patch).is_err());
        }
    }

    #[test]
    fn translation_only_needs_a_translation_target() {
        let hide = |translate_to: Vec<LangCode>| Settings {
            translate_to,
            engines: Engines::OPENAI,
            subtitle_display: SubtitleDisplay {
                show_source: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let only = hide(vec!["en".into()]);
        assert!(only.plan().translation_only);
        assert_eq!(only.plan().partial.text, None);
        assert_eq!(only.plan().finals.ruby, None);

        let source = hide(Vec::new());
        assert!(!source.plan().translation_only);
        assert_eq!(
            source.plan().partial.text,
            Some(TextSource::GptLiveTranscribe)
        );
        assert_eq!(source.plan().finals.ruby, Some(RubySource::Llm));
    }

    fn plan_of(engines: Engines) -> Plan {
        Settings {
            engines,
            translate_to: vec!["en".into()],
            deadlock_mode: true,
            ..Default::default()
        }
        .plan()
    }

    /// README の「しくみ」の表と同じ値。表を変えたら、ここも変える。
    /// 0.4 までの 3 つのモードと同じ作り方になる
    #[test]
    fn the_plan_matches_the_table_in_the_readme() {
        use EmphasisSource as E;
        use RubySource as R;
        use TextSource as T;
        use TranslationSource as Tr;
        let phase = |text, translation, ruby, emphasis| Phase {
            text: Some(text),
            translation,
            ruby: Some(ruby),
            emphasis: Some(emphasis),
        };
        let chrome = phase(
            T::Chrome,
            Some(Tr::ChromeTranslator),
            R::Dictionary,
            E::Terms,
        );
        let llm = phase(T::GptTranscribe, Some(Tr::Llm), R::Llm, E::Llm);
        let live = phase(T::GptLiveTranscribe, None, R::Dictionary, E::Terms);

        let recommended = plan_of(Engines::STANDARD);
        assert_eq!((recommended.partial, recommended.finals), (chrome, llm));
        let openai = plan_of(Engines::OPENAI);
        assert_eq!((openai.partial, openai.finals), (live, llm));
        let free = plan_of(Engines::CHROME);
        assert_eq!((free.partial, free.finals), (chrome, chrome));
    }

    #[test]
    fn each_engine_changes_only_its_part_of_the_finals() {
        use EmphasisSource as E;
        use RubySource as R;
        use TextSource as T;
        use TranslationSource as Tr;
        let llm_text = plan_of(Engines {
            transcription: FinalSource::GptTranscribe,
            ..Engines::CHROME
        });
        assert_eq!(llm_text.finals.text, Some(T::GptTranscribe));
        assert_eq!(llm_text.finals.translation, Some(Tr::ChromeTranslator));
        assert_eq!(llm_text.finals.emphasis, Some(E::Terms));
        assert_eq!(llm_text.finals.ruby, Some(R::Dictionary));

        let llm_words = plan_of(Engines {
            translation: TranslationEngine::Gpt61Terra,
            furigana: R::Llm,
            ..Engines::CHROME
        });
        assert_eq!(llm_words.finals.text, Some(T::Chrome));
        assert_eq!(llm_words.finals.translation, Some(Tr::Llm));
        assert_eq!(llm_words.finals.emphasis, Some(E::Llm));
        assert_eq!(llm_words.finals.ruby, Some(R::Llm));
        assert_eq!(llm_words.partial, plan_of(Engines::CHROME).partial);
    }

    /// ふりがなを判定モデルに選ばせるなら、OpenRouter を使う（ベータの中継で足りる）。OpenAI は要らない
    #[test]
    fn decider_furigana_uses_openrouter_and_starts_from_the_dictionary() {
        let chosen = Settings {
            engines: Engines {
                furigana: RubySource::Decider,
                ..Engines::CHROME
            },
            ..Default::default()
        };
        assert_eq!(chosen.plan().finals.ruby, Some(RubySource::Decider));
        // 確定の文にも、まず辞書の読みを付ける
        assert!(chosen.plan().finals.marks());
        assert_eq!(chosen.engines.providers(), [Provider::OpenRouter]);
        assert_eq!(Engines::BETA.furigana, RubySource::Decider);
        assert!(!Engines::BETA.uses(Provider::OpenAi));
    }

    /// ふりがなを GPT-6 Luna にすると、読みを直すモデルだけが変わる（辞書の名前の読みは `models.readings` のまま）
    #[test]
    fn luna_furigana_changes_only_the_model_of_readings() {
        let luna = Settings {
            engines: Engines {
                furigana: RubySource::Luna,
                ..Engines::CHROME
            },
            ..Default::default()
        };
        assert_eq!(luna.plan().finals.ruby, Some(RubySource::Luna));
        assert_eq!(luna.plan().partial.ruby, Some(RubySource::Dictionary));
        assert_eq!(luna.models_in_use().readings, "gpt-6-luna");
        assert_eq!(luna.models.readings, Models::default().readings);
        assert!(luna.engines.uses(Provider::OpenAi));
        // 直すもとの辞書の読みは日本語だけ
        let english = Settings {
            mic_language: "en".into(),
            ..luna
        };
        assert_eq!(english.plan().finals.ruby, None);
    }

    #[test]
    fn chrome_and_keys_follow_the_engines() {
        assert!(plan_of(Engines::STANDARD).uses_chrome());
        assert!(Engines::STANDARD.paid());
        assert!(plan_of(Engines::CHROME).uses_chrome());
        assert!(!Engines::CHROME.paid());
        assert!(!plan_of(Engines::OPENAI).uses_chrome());
        assert!(Engines::OPENAI.paid());
        let paid_furigana = Engines {
            furigana: RubySource::Llm,
            ..Engines::CHROME
        };
        assert!(plan_of(paid_furigana).uses_chrome());
        assert!(paid_furigana.paid());
        assert!(paid_furigana.uses(Provider::OpenAi) && !paid_furigana.uses(Provider::Xai));
    }

    /// 用語と翻訳を分けた作り方（DESIGN.md 9.7）。翻訳が LLM なら、用語の選択は使わない
    #[test]
    fn terms_and_translation_are_chosen_apart_unless_the_llm_does_both() {
        use EmphasisSource as E;
        use TranslationSource as Tr;
        let split = Engines {
            terms: TermsEngine::Decisions,
            translation: TranslationEngine::QwenMtFlash,
            ..Engines::CHROME
        };
        assert_eq!(split.sanitized(), split);
        assert!(
            split.decides_terms() && split.uses(Provider::OpenAi) && split.uses(Provider::Qwen)
        );
        assert!(!split.uses(Provider::Xai));
        let finals = plan_of(split).finals;
        assert_eq!(
            (finals.emphasis, finals.translation),
            (Some(E::Decisions), Some(Tr::Mt))
        );
        // 確かめられなかったときのために、PC の中の一致も付ける
        assert!(finals.marks() && finals.local_terms());
        // 途中の字幕は今までどおり
        assert_eq!(plan_of(split).partial, plan_of(Engines::CHROME).partial);

        // 用語だけ Decisions、翻訳は Chrome
        let decided = Engines {
            terms: TermsEngine::Decisions,
            ..Engines::CHROME
        };
        assert!(decided.uses(Provider::OpenAi) && !decided.uses(Provider::Qwen));
        let finals = plan_of(decided).finals;
        assert_eq!(
            (finals.emphasis, finals.translation),
            (Some(E::Decisions), Some(Tr::ChromeTranslator))
        );

        // 翻訳だけ Qwen、用語は辞書の一致（OpenAI は使わない）
        let translated = Engines {
            translation: TranslationEngine::QwenMtFlash,
            ..Engines::CHROME
        };
        assert!(
            translated.paid()
                && translated.uses(Provider::Qwen)
                && !translated.uses(Provider::OpenAi)
        );
        assert_eq!(plan_of(translated).finals.emphasis, Some(E::Terms));

        // LLM が翻訳と同時に判定する
        let llm = Engines {
            terms: TermsEngine::Decisions,
            ..Engines::STANDARD
        };
        assert!(!llm.decides_terms());
        assert_eq!(plan_of(llm), plan_of(Engines::STANDARD));
    }

    /// Gemini Nano は Chrome のページの中にあるので、途中の字幕が Chrome のときだけ使える。鍵は要らない
    #[test]
    fn gemini_nano_judges_terms_only_with_the_chrome_page() {
        let nano = Engines {
            terms: TermsEngine::GeminiNano,
            ..Engines::CHROME
        };
        assert_eq!(nano.sanitized(), nano);
        assert_eq!(plan_of(nano).finals.emphasis, Some(EmphasisSource::Chrome));
        // 途中の字幕は、今までどおり辞書の一致
        assert_eq!(plan_of(nano).partial.emphasis, Some(EmphasisSource::Terms));
        assert!(plan_of(nano).finals.local_terms());
        assert!(nano.providers().is_empty());
        let without_chrome = Engines {
            terms: TermsEngine::GeminiNano,
            live: LiveSource::HaasJa,
            transcription: FinalSource::HaasJa,
            ..Engines::BETA
        };
        assert_eq!(without_chrome.sanitized().terms, TermsEngine::Dictionary);
    }

    /// 候補の出し方は日本語だけなので、ほかの言語では Decisions に聞かない
    #[test]
    fn decisions_are_asked_only_for_japanese() {
        let settings = Settings {
            engines: Engines {
                terms: TermsEngine::Decisions,
                ..Engines::CHROME
            },
            mic_language: "en".into(),
            translate_to: vec!["ja".into()],
            ..Default::default()
        };
        assert_eq!(settings.plan().finals.emphasis, None);
    }

    /// 0.5 までの設定ファイルには用語の選択がない。辞書の一致として読む
    #[test]
    fn engines_without_terms_read_as_the_dictionary() {
        let engines: Engines = serde_json::from_value(json!({
            "live": "chrome",
            "transcription": "gptTranscribe",
            "translation": "gpt61Terra",
            "furigana": "lindera",
        }))
        .unwrap();
        assert_eq!(engines.terms, TermsEngine::Dictionary);
        assert_eq!(engines.translation, TranslationEngine::Gpt61Terra);
    }

    /// 消した「GPT-6 Luna（用語も）」は、ベータの用語と翻訳として読む（0.5 のライトの人）。ほかの工程は変えない
    #[test]
    fn luna_reads_as_the_terms_and_translation_of_the_beta() {
        let engines: Engines = serde_json::from_value(json!({
            "live": "chrome",
            "transcription": "grokVoiceTranscribe",
            "translation": "gpt6Luna",
            "furigana": "lindera",
        }))
        .unwrap();
        assert_eq!(
            engines,
            Engines {
                transcription: FinalSource::GrokVoiceTranscribe,
                furigana: RubySource::Dictionary,
                ..Engines::BETA
            }
        );
        assert!(serde_json::from_value::<Engines>(json!({ "translation": "unknown" })).is_err());
    }

    /// Grok の確定は xAI だけを使い、確定の文の出どころとモデルだけが変わる
    /// 画面と LP の一覧を書き出す（ts-rs の型と同じく、テストで作る）。中身が同じなら触らない
    #[test]
    fn export_catalog() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../frontend/src/generated/catalog.ts"
        );
        let catalog = catalog();
        if std::fs::read_to_string(path).ok().as_deref() != Some(&catalog) {
            std::fs::write(path, &catalog).unwrap();
        }
        // 選択肢を足したら `ALL` にも足す（足し忘れると、画面の一覧に会社が出ない）
        assert!(catalog.contains("\"haasJa\": null") && catalog.contains("\"jev\": \"typesafe\""));
    }

    #[test]
    fn each_step_asks_for_the_key_of_its_company() {
        // 用語は Jev、翻訳は Hy-MT2、確定は MAI-Transcribe 2（OpenAI は使わない）
        let mixed = Engines {
            transcription: FinalSource::MaiTranscribe2,
            terms: TermsEngine::Jev,
            translation: TranslationEngine::HyMt2,
            ..Engines::CHROME
        };
        assert_eq!(
            mixed.providers(),
            [Provider::OpenRouter, Provider::TypeSafe]
        );
        assert_eq!(mixed.judge().unwrap().name, "jev-latest");
        assert_eq!(
            mixed.translation.mt().unwrap().name,
            "tencent/hy-mt2-30b-a3b"
        );
        let settings = Settings {
            engines: mixed,
            ..Settings::default()
        };
        assert_eq!(settings.models_in_use().batch, "microsoft/mai-transcribe-2");
        // 翻訳が LLM なら、判定モデルは使わない（その会社の鍵も要らない）
        let llm = Engines {
            terms: TermsEngine::DrexMax,
            ..Engines::STANDARD
        };
        assert_eq!(llm.providers(), [Provider::OpenAi]);
        // PC の中の文字起こしは鍵が要らない
        let local = Engines {
            transcription: FinalSource::HaasJa,
            ..Engines::CHROME
        };
        assert!(!local.paid());
        assert_eq!(
            Settings {
                engines: local,
                ..Settings::default()
            }
            .models_in_use()
            .batch,
            crate::local::HAAS_JA
        );
    }

    #[test]
    fn grok_uses_xai_for_the_finals() {
        let grok = Engines {
            transcription: FinalSource::GrokVoiceTranscribe,
            ..Engines::CHROME
        };
        assert!(grok.paid() && grok.uses(Provider::Xai) && !grok.uses(Provider::OpenAi));
        assert_eq!(grok.sanitized(), grok);
        let settings = Settings {
            engines: grok,
            ..Default::default()
        };
        assert_eq!(
            settings.plan().finals.text,
            Some(TextSource::GrokVoiceTranscribe)
        );
        assert_eq!(settings.plan().partial, plan_of(Engines::STANDARD).partial);
        assert_eq!(settings.models_in_use().batch, "grok-voice-transcribe-2.0");
        assert_eq!(Settings::default().models_in_use().batch, "gpt-transcribe");
    }

    #[test]
    fn engines_that_cannot_work_together_are_fixed() {
        // 確定の文が Chrome なら、発話の区切りも Chrome
        let chrome_finals = Engines {
            live: LiveSource::Off,
            ..Engines::CHROME
        };
        assert_eq!(chrome_finals.sanitized(), Engines::CHROME);
        // Chrome の翻訳は Chrome のページの中で動く
        let no_page = Engines {
            translation: TranslationEngine::ChromeTranslator,
            ..Engines::OPENAI
        };
        assert_eq!(no_page.sanitized(), Engines::OPENAI);
        let patch = SettingsPatch {
            engines: Some(no_page),
            ..Default::default()
        };
        let next = Settings::default().apply(patch).unwrap();
        assert_eq!(next.engines, Engines::OPENAI);
    }

    #[test]
    fn the_modes_of_0_4_become_engines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let read = |json: Value| {
            std::fs::write(&path, json.to_string()).unwrap();
            load(&path).unwrap().engines
        };
        assert_eq!(read(json!({ "speechEngine": "hybrid" })), Engines::STANDARD);
        for saved in ["chrome", "local", "free"] {
            assert_eq!(read(json!({ "speechEngine": saved })), Engines::CHROME);
        }
        assert_eq!(read(json!({ "speechEngine": "openai" })), Engines::OPENAI);
        assert_eq!(
            read(json!({ "speechEngine": "openai", "liveSubtitles": false })),
            Engines {
                live: LiveSource::Off,
                ..Engines::OPENAI
            }
        );
        // 保存し直すと、モードは残らない
        let s = load(&path).unwrap();
        save(&path, &s).unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(saved.get("speechEngine").is_none());
        assert!(saved.get("liveSubtitles").is_none());
        assert_eq!(load(&path).unwrap().engines, s.engines);
    }

    #[test]
    fn settings_turn_items_off_in_the_plan() {
        let with = |change: fn(&mut Settings)| {
            let mut settings = Settings {
                engines: Engines::CHROME,
                translate_to: vec!["en".into()],
                deadlock_mode: true,
                ..Default::default()
            };
            change(&mut settings);
            settings.plan()
        };
        let no_furigana = with(|s| s.subtitle_display.furigana = false);
        assert_eq!(no_furigana.partial.ruby, None);
        let no_terms = with(|s| s.deadlock_mode = false);
        assert_eq!(no_terms.finals.emphasis, None);
        let no_targets = with(|s| s.translate_to.clear());
        assert_eq!(no_targets.partial.translation, None);
        let english = with(|s| s.mic_language = "en".into());
        assert_eq!(english.partial.ruby, None);
        assert_eq!(english.partial.emphasis, None);
        assert!(english.partial.text.is_some(), "Chrome は認識できる");
        let translation_only = with(|s| s.subtitle_display.show_source = false);
        assert_eq!(translation_only.partial.ruby, None);
        assert!(
            translation_only.partial.emphasis.is_some(),
            "訳の用語に要る"
        );
    }

    #[test]
    fn chrome_is_only_for_new_users() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(load(&path).unwrap().engines, Engines::CHROME);
        // 0.3 より前の設定ファイルには `speechEngine` がない
        std::fs::write(&path, r#"{ "micLanguage": "ja" }"#).unwrap();
        assert_eq!(load(&path).unwrap().engines, Engines::OPENAI);
        // 選んだものは保存される
        let recommended = Settings {
            engines: Engines::STANDARD,
            ..Default::default()
        };
        save(&path, &recommended).unwrap();
        assert_eq!(load(&path).unwrap().engines, Engines::STANDARD);
    }

    #[test]
    fn english_rank_names_are_only_for_new_users() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let new_user = load(&path).unwrap();
        assert_eq!(new_user.rank_language, Some("en".into()));
        assert_eq!(new_user.rank_language(), "en");
        // 前の版の設定ファイルには `rankLanguage` がない。話す言語に合わせたまま
        std::fs::write(&path, r#"{ "micLanguage": "ja" }"#).unwrap();
        let existing = load(&path).unwrap();
        assert_eq!(existing.rank_language, None);
        assert_eq!(existing.rank_language(), "ja");
        // 「話す言語に合わせる」を選んだ人も、保存して読み直して変わらない
        let follow = new_user
            .apply(SettingsPatch {
                rank_language: Some(String::new()),
                ..Default::default()
            })
            .unwrap();
        save(&path, &follow).unwrap();
        assert_eq!(load(&path).unwrap().rank_language, None);
        // 選んだ言語はそのまま
        let korean = Settings {
            rank_language: Some("ko".into()),
            ..Default::default()
        };
        save(&path, &korean).unwrap();
        assert_eq!(load(&path).unwrap().rank_language, Some("ko".into()));
    }

    #[test]
    fn font_size_goes_down_to_12() {
        let patch = |size| SettingsPatch {
            subtitle_display: Some(SubtitleDisplayPatch {
                font_size: Some(size),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = Settings::default();
        assert!(s.apply(patch(12)).is_ok());
        assert!(s.apply(patch(11)).is_err());
    }

    #[test]
    fn translate_to_drops_the_spoken_language_and_unknown_codes() {
        let s = Settings::default()
            .apply(SettingsPatch {
                translate_to: Some(["ja", "en", "xx", "en", "ru"].map(String::from).to_vec()),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(s.translate_to, ["en", "ru"]);
        // 話す言語を英語に変えたら、翻訳先から英語を外す
        let s = s
            .apply(SettingsPatch {
                mic_language: Some("en".into()),
                ..SettingsPatch::default()
            })
            .unwrap();
        assert_eq!(s.translate_to, ["ru"]);
    }

    #[test]
    fn missing_fields_take_defaults_and_broken_files_are_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{ "micLanguage": "en" }"#).unwrap();
        let s = load(&path).unwrap();
        assert_eq!(s.mic_language, "en");
        assert_eq!(s.rank_poll_seconds, 180);

        std::fs::write(&path, "{ broken").unwrap();
        assert_eq!(load(&path).unwrap(), Settings::default());
        assert!(dir.path().join("settings.json.broken").exists());
    }

    #[test]
    fn hand_edited_files_are_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{ "micLanguage": "xx", "translateTo": ["ja", "en", "ru", "ko", "de", "fr"] }"#,
        )
        .unwrap();
        let s = load(&path).unwrap();
        assert_eq!(s.mic_language, "ja");
        assert_eq!(s.translate_to, ["en", "ru", "ko", "de"]);
    }

    #[test]
    fn old_voxtral_models_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{ "models": { "realtime": "voxtral-mini-transcribe-realtime-3", "batch": "voxtral-small-transcribe-3", "translate": "custom" } }"#,
        )
        .unwrap();
        let models = load(&path).unwrap().models;
        assert_eq!(models.realtime, Models::default().realtime);
        assert_eq!(models.batch, Models::default().batch);
        assert_eq!(models.translate, "custom");
    }

    #[test]
    fn broken_display_values_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{ "subtitleDisplay": { "fontSize": 200 } }"#).unwrap();
        assert_eq!(
            load(&path).unwrap().subtitle_display,
            SubtitleDisplay::default()
        );
    }

    #[test]
    fn display_patch_changes_only_what_it_names() {
        let next = Settings::default()
            .apply(SettingsPatch {
                subtitle_display: Some(SubtitleDisplayPatch {
                    align: Some(Align::Left),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(next.subtitle_display.align, Align::Left);
        assert_eq!(
            next.subtitle_display.font_size,
            SubtitleDisplay::default().font_size
        );
    }

    #[test]
    fn display_seconds_stay_in_range() {
        let apply = |patch| {
            Settings::default().apply(SettingsPatch {
                subtitle_display: Some(patch),
                ..Default::default()
            })
        };
        let wait = |s| SubtitleDisplayPatch {
            max_wait_seconds: Some(s),
            ..Default::default()
        };
        let read = |s| SubtitleDisplayPatch {
            min_read_seconds: Some(s),
            ..Default::default()
        };
        let per_char = |s| SubtitleDisplayPatch {
            per_char_seconds: Some(s),
            ..Default::default()
        };
        assert!(apply(wait(0.0)).is_ok());
        assert!(apply(wait(31.0)).is_err());
        assert!(apply(read(1.0)).is_ok());
        assert!(apply(read(0.5)).is_err());
        assert!(apply(per_char(1.0)).is_ok());
        assert!(apply(per_char(1.5)).is_err());
    }

    #[test]
    fn unknown_display_fields_do_not_reset_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r##"{ "subtitleDisplay": { "textColor": "#ffe066", "fontSize": 32 } }"##,
        )
        .unwrap();
        assert_eq!(load(&path).unwrap().subtitle_display.font_size, 32);
    }

    #[test]
    fn unreadable_display_keeps_the_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{ "rankAccount": 308386819, "subtitleDisplay": { "align": "none" } }"#,
        )
        .unwrap();
        let s = load(&path).unwrap();
        assert_eq!(s.rank_account, Some(308_386_819));
        assert_eq!(s.subtitle_display, SubtitleDisplay::default());
    }

    #[test]
    fn broken_item_cards_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{ "itemCards": { "holdSeconds": 0 } }"#).unwrap();
        assert_eq!(load(&path).unwrap().item_cards, ItemCards::default());
        std::fs::write(
            &path,
            r#"{ "itemCards": { "holdSeconds": 12, "language": "en" } }"#,
        )
        .unwrap();
        assert_eq!(load(&path).unwrap().item_language(), "en");

        // 言語を選んでいなくても、秒数は残る
        std::fs::write(&path, r#"{ "itemCards": { "holdSeconds": 12 } }"#).unwrap();
        let s = load(&path).unwrap();
        assert_eq!(
            (s.item_cards.hold_seconds, s.item_language().as_str()),
            (12.0, "ja")
        );
    }

    fn subtitle_style(patch: serde_json::Value) -> SettingsPatch {
        SettingsPatch {
            subtitle_style: Some(patch),
            ..Default::default()
        }
    }

    fn rank_style(patch: serde_json::Value) -> SettingsPatch {
        SettingsPatch {
            rank_style: Some(patch),
            ..Default::default()
        }
    }

    fn font_of(s: &Settings, language: &str) -> Option<String> {
        s.subtitle_style
            .languages
            .get(language)
            .and_then(|f| f.font.clone())
    }

    #[test]
    fn a_style_patch_changes_only_what_it_names() {
        let s = Settings::default()
            .apply(subtitle_style(json!({
                "color": "#ffe066",
                "outline": { "width": 0.3, "color": "#102030" },
                "languages": {
                    "ja": { "font": "Hiragino Mincho ProN", "weight": 500 },
                    "en": { "font": "Georgia" }
                }
            })))
            .unwrap();
        let next = s
            .apply(subtitle_style(json!({
                "outline": { "width": 0.1 },
                "languages": { "ja": { "letterSpacing": 0.05 } }
            })))
            .unwrap();
        assert_eq!(next.subtitle_style.look.color.as_deref(), Some("#ffe066"));
        assert_eq!(next.subtitle_style.look.outline.width, Some(0.1));
        assert_eq!(
            next.subtitle_style.look.outline.color.as_deref(),
            Some("#102030")
        );
        let ja = &next.subtitle_style.languages["ja"];
        assert_eq!(ja.font.as_deref(), Some("Hiragino Mincho ProN"));
        assert_eq!((ja.weight, ja.letter_spacing), (Some(500), Some(0.05)));
        assert_eq!(font_of(&next, "en").as_deref(), Some("Georgia"));
        assert_eq!(next.rank_style, RankStyle::default());
    }

    #[test]
    fn null_returns_a_style_item_to_the_standard() {
        let s = Settings::default()
            .apply(subtitle_style(json!({
                "color": "#ffe066",
                "shadow": { "enabled": false, "blur": 0.4 },
                "lineHeight": 2.0,
                "background": { "enabled": true, "opacity": 0.3 },
                "languages": { "ja": { "font": "Hiragino Sans" }, "en": { "font": "Georgia" } }
            })))
            .unwrap();
        let next = s
            .apply(subtitle_style(json!({
                "color": null,
                "shadow": { "blur": null },
                "languages": { "ja": { "font": null } }
            })))
            .unwrap();
        assert_eq!(next.subtitle_style.look.color, None);
        assert_eq!(next.subtitle_style.look.shadow.enabled, Some(false));
        assert_eq!(next.subtitle_style.look.shadow.blur, None);
        assert!(!next.subtitle_style.languages.contains_key("ja"));
        assert_eq!(font_of(&next, "en").as_deref(), Some("Georgia"));

        let reset = next
            .apply(subtitle_style(json!({
                "color": null, "outline": null, "shadow": null, "lineHeight": null,
                "background": null, "languages": null
            })))
            .unwrap();
        assert_eq!(reset.subtitle_style, SubtitleStyle::default());
    }

    #[test]
    fn rank_style_takes_one_font_and_leaves_the_subtitle_style_alone() {
        let s = Settings::default()
            .apply(subtitle_style(
                json!({ "languages": { "ja": { "font": "Hiragino Sans" } } }),
            ))
            .unwrap();
        let next = s
            .apply(rank_style(json!({
                "font": "Georgia", "weight": 900, "letterSpacing": 0.02,
                "color": "#ffe066"
            })))
            .unwrap();
        assert_eq!(next.rank_style.font.font.as_deref(), Some("Georgia"));
        assert_eq!(next.rank_style.font.weight, Some(900));
        assert_eq!(next.rank_style.look.color.as_deref(), Some("#ffe066"));
        assert_eq!(next.subtitle_style, s.subtitle_style);
        let back = next.apply(rank_style(json!({ "font": null }))).unwrap();
        assert_eq!(back.rank_style.font.font, None);
        assert_eq!(back.rank_style.font.weight, Some(900));
    }

    #[test]
    fn any_weight_css_accepts_can_be_saved() {
        for weight in [1, 950, 1000] {
            let s = Settings::default()
                .apply(subtitle_style(
                    json!({ "languages": { "ja": { "weight": weight } } }),
                ))
                .unwrap()
                .apply(rank_style(json!({ "weight": weight })))
                .unwrap();
            assert_eq!(s.subtitle_style.languages["ja"].weight, Some(weight));
            assert_eq!(s.rank_style.font.weight, Some(weight));
        }
    }

    #[test]
    fn style_patches_reject_values_out_of_range_and_change_nothing() {
        let s = Settings::default();
        for bad in [
            json!({ "color": "red" }),
            json!({ "color": "#ffe06" }),
            json!({ "color": "#ffe06g" }),
            json!({ "color": "#ffffffff" }),
            json!({ "color": "#ｆｆ" }),
            json!({ "color": 1 }),
            json!({ "outline": { "width": 0.51 } }),
            json!({ "outline": { "width": -0.1 } }),
            json!({ "outline": { "color": "url(x)" } }),
            json!({ "shadow": { "blur": 1.1 } }),
            json!({ "shadow": { "color": "x" } }),
            json!({ "shadow": { "enabled": "yes" } }),
            json!({ "lineHeight": 0.9 }),
            json!({ "lineHeight": 3.1 }),
            json!({ "background": { "opacity": 1.1 } }),
            json!({ "background": { "color": "black" } }),
            json!({ "languages": { "latin": { "font": "Georgia" } } }),
            json!({ "languages": { "ja": { "weight": 0 } } }),
            json!({ "languages": { "ja": { "weight": 1001 } } }),
            json!({ "languages": { "ja": { "letterSpacing": 0.6 } } }),
            json!({ "languages": { "ja": { "letterSpacing": -0.2 } } }),
            json!({ "languages": { "ja": { "font": "a\nb" } } }),
        ] {
            assert!(s.apply(subtitle_style(bad.clone())).is_err(), "{bad}");
        }
        for bad in [
            json!({ "color": "red" }),
            json!({ "font": "a\nb" }),
            json!({ "weight": 1001 }),
            json!({ "letterSpacing": 0.11 }),
        ] {
            assert!(s.apply(rank_style(bad.clone())).is_err(), "{bad}");
        }
    }

    #[test]
    fn font_names_keep_the_length_and_control_character_rules() {
        let s = Settings::default();
        let font = |name: &str| subtitle_style(json!({ "languages": { "ja": { "font": name } } }));
        for good in ["あ".repeat(256), "a\"b".to_owned(), "a\\b".to_owned()] {
            assert!(s.apply(font(&good)).is_ok(), "{good}");
        }
        for bad in ["あ".repeat(257), "a\u{7f}b".to_owned(), String::new()] {
            assert!(s.apply(font(&bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_standard_style_has_no_values_for_the_overlays_to_apply() {
        let json = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(
            json["subtitleStyle"],
            json!({
                "color": null,
                "outline": { "width": null, "color": null },
                "shadow": { "enabled": null, "blur": null, "color": null },
                "background": { "enabled": false, "color": null, "opacity": null },
                "lineHeight": null,
                "languages": {}
            })
        );
        assert_eq!(
            json["rankStyle"],
            json!({
                "color": null,
                "outline": { "width": null, "color": null },
                "shadow": { "enabled": null, "blur": null, "color": null },
                "font": null,
                "weight": null,
                "letterSpacing": null
            })
        );
    }

    #[test]
    fn band_and_fonts_from_0_3_2_go_back_to_the_standard() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            json!({
                "speechEngine": "chrome",
                "micLanguage": "ko",
                "rankAccount": 308_386_819,
                "overlayFonts": { "ja": "Old Font" },
                "subtitleDisplay": {
                    "align": "left", "fontSize": 52, "translationFontSize": 30,
                    "background": "band", "maxLines": 2, "holdSeconds": 9
                }
            })
            .to_string(),
        )
        .unwrap();
        let s = load(&path).unwrap();
        assert_eq!(s.engines, Engines::CHROME);
        assert_eq!(s.mic_language, "ko");
        assert_eq!(s.rank_account, Some(308_386_819));
        assert_eq!(
            s.subtitle_display,
            SubtitleDisplay {
                align: Align::Left,
                font_size: 52,
                translation_font_size: 30,
                max_lines: 2,
                hold_seconds: 9.0,
                ..Default::default()
            }
        );
        assert_eq!(s.subtitle_style, SubtitleStyle::default());
        assert_eq!(s.rank_style, RankStyle::default());

        save(&path, &s).unwrap();
        let file = std::fs::read_to_string(&path).unwrap();
        assert!(!file.contains("overlayFonts") && !file.contains("band"));
        assert_eq!(load(&path).unwrap(), s);
    }
    #[test]
    fn broken_style_values_fall_back_without_losing_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r##"{
                "subtitleDisplay": { "fontSize": 200, "maxLines": 2 },
                "subtitleStyle": {
                    "color": "red",
                    "lineHeight": 9,
                    "outline": { "width": 9, "color": "#102030" },
                    "shadow": { "enabled": "yes", "blur": 0.5 },
                    "background": { "enabled": true, "opacity": 2 },
                    "languages": {
                        "ja": { "font": "Hiragino Sans", "weight": 500 },
                        "zh-Hans": { "font": "bad\nname", "letterSpacing": 0.2 },
                        "ko": 1,
                        "en": { "weight": 5000 },
                        "latin": { "font": "Georgia" }
                    }
                },
                "rankStyle": {
                    "font": "bad\nname",
                    "weight": 500,
                    "letterSpacing": 0.3,
                    "color": "#ffe066",
                    "outline": { "color": 1 }
                },
                "itemCards": { "holdSeconds": 0, "language": "en" }
            }"##,
        )
        .unwrap();
        let s = load(&path).unwrap();
        assert_eq!(s.subtitle_display.font_size, 40);
        assert_eq!(s.subtitle_display.max_lines, 2);
        assert_eq!(
            s.subtitle_style,
            SubtitleStyle {
                look: TextLook {
                    outline: Outline {
                        color: Some("#102030".to_owned()),
                        ..Default::default()
                    },
                    shadow: Shadow {
                        blur: Some(0.5),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                background: Background {
                    enabled: true,
                    ..Default::default()
                },
                languages: BTreeMap::from([
                    (
                        "ja".to_owned(),
                        FontStyle {
                            font: Some("Hiragino Sans".to_owned()),
                            weight: Some(500),
                            letter_spacing: None,
                        }
                    ),
                    (
                        "zh-Hans".to_owned(),
                        FontStyle {
                            letter_spacing: Some(0.2),
                            ..Default::default()
                        }
                    ),
                ]),
                ..Default::default()
            }
        );
        assert_eq!(
            s.rank_style.font,
            FontStyle {
                weight: Some(500),
                ..Default::default()
            }
        );
        assert_eq!(s.rank_style.look.color.as_deref(), Some("#ffe066"));
        assert_eq!(s.item_cards.hold_seconds, ItemCards::default().hold_seconds);
        assert_eq!(s.item_cards.language.as_deref(), Some("en"));
        // 残った値は変更の検証を通る
        s.apply(subtitle_style(json!({ "color": "#ffffff" })))
            .unwrap();
    }

    #[test]
    fn the_rank_band_from_0_4_0_rc_is_dropped_and_the_rest_stays() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        std::fs::write(
            &path,
            r##"{
                "subtitleStyle": { "background": { "enabled": true, "opacity": 0.4 } },
                "rankStyle": {
                    "font": "Georgia", "weight": 900, "color": "#ffe066",
                    "background": { "enabled": true, "color": "#112233", "opacity": 0.5 }
                }
            }"##,
        )
        .unwrap();
        let s = load(&path).unwrap();
        assert_eq!(s.rank_style.font.font.as_deref(), Some("Georgia"));
        assert_eq!(s.rank_style.font.weight, Some(900));
        assert_eq!(s.rank_style.look.color.as_deref(), Some("#ffe066"));
        assert_eq!(
            s.subtitle_style.background,
            Background {
                enabled: true,
                color: None,
                opacity: Some(0.4)
            }
        );

        save(&path, &s).unwrap();
        let file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(file["rankStyle"].get("background").is_none());
        assert_eq!(load(&path).unwrap(), s);
        // 操作パネルから送っても、ランクには帯を持てない
        let next = s
            .apply(rank_style(json!({ "background": { "enabled": true } })))
            .unwrap();
        assert_eq!(next.rank_style, s.rank_style);
    }

    #[test]
    fn an_unknown_rank_content_falls_back_to_the_gauge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        std::fs::write(
            &path,
            r#"{ "rankAccount": 308386819, "rankContent": "full", "rankAnchor": "bottom-right" }"#,
        )
        .unwrap();
        let s = load(&path).unwrap();
        assert_eq!(s.rank_content, RankContent::Gauge);
        assert_eq!(s.rank_account, Some(308_386_819));
        assert_eq!(s.rank_anchor, Anchor::BottomRight);
    }

    #[test]
    fn an_unreadable_style_keeps_the_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{ "rankAccount": 308386819, "subtitleStyle": { "color": 1 }, "rankStyle": 1 }"#,
        )
        .unwrap();
        let s = load(&path).unwrap();
        assert_eq!(s.rank_account, Some(308_386_819));
        assert_eq!(s.subtitle_style, SubtitleStyle::default());
        assert_eq!(s.rank_style, RankStyle::default());
    }

    #[test]
    fn chosen_styles_survive_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let s = Settings::default()
            .apply(subtitle_style(json!({
                "outline": { "width": 0.07 },
                "languages": { "zh-Hant": { "font": "PingFang TC", "letterSpacing": 0.03 } }
            })))
            .unwrap()
            .apply(rank_style(
                json!({ "shadow": { "blur": 0.1 }, "font": "Georgia" }),
            ))
            .unwrap();
        save(&path, &s).unwrap();
        assert_eq!(load(&path).unwrap(), s);
    }

    #[test]
    fn rank_patch_sets_content_and_region() {
        let s = Settings::default();
        let next = s
            .apply(SettingsPatch {
                rank_content: Some(RankContent::Icon),
                rank_region: Some("Asia".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(next.rank_content, RankContent::Icon);
        assert_eq!(next.rank_region, Some(Region::Asia));
        let off = next
            .apply(SettingsPatch {
                rank_region: Some(String::new()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(off.rank_region, None);
        assert!(
            s.apply(SettingsPatch {
                rank_region: Some("Mars".into()),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[test]
    fn item_cards_patch_validates_and_follows_the_spoken_language() {
        let s = Settings::default();
        assert_eq!(s.item_language(), "ja");
        let next = s
            .apply(SettingsPatch {
                item_cards: Some(ItemCardsPatch {
                    hold_seconds: Some(12.0),
                    language: Some("en".into()),
                    anchor: Some(Anchor::Center),
                }),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(next.item_cards.hold_seconds, 12.0);
        assert_eq!(next.item_language(), "en");
        assert_eq!(next.item_cards.anchor, Anchor::Center);
        for bad in [
            ItemCardsPatch {
                hold_seconds: Some(1.0),
                ..Default::default()
            },
            ItemCardsPatch {
                language: Some("xx".into()),
                ..Default::default()
            },
        ] {
            assert!(
                s.apply(SettingsPatch {
                    item_cards: Some(bad),
                    ..Default::default()
                })
                .is_err()
            );
        }
    }

    #[test]
    fn only_customized_advanced_fields_are_saved() {
        let dir = std::env::temp_dir().join(format!("overlays-settings-{}", std::process::id()));
        let path = dir.join(FILE);
        let saved = |s: &Settings| {
            save(&path, s).unwrap();
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap()
        };
        let json = saved(&Settings::default());
        for key in ADVANCED {
            assert!(json.get(key).is_none(), "{key}");
        }
        assert!(json.get("translateTo").is_some());

        let mut s = Settings::default();
        s.models.translate = "custom".into();
        assert!(saved(&s).get("models").is_some());
        assert_eq!(load(&path).unwrap(), s);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
