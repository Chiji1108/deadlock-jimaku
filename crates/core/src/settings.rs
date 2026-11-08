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
    /// 音声認識と翻訳をどこで動かすか
    pub speech_engine: SpeechEngine,
    /// 配信者が話す言語
    pub mic_language: LangCode,
    /// マイクのデバイス名。None なら OS の既定の入力デバイス
    #[ts(optional)]
    pub mic_device: Option<String>,
    /// 翻訳先（最大 4 言語。mic_language は除く）
    pub translate_to: Vec<LangCode>,
    /// ヒーロー・スキル・アイテムを判定して強調し、アイテム説明を出す。雑談の配信ならオフにする
    pub deadlock_mode: bool,
    /// 話している途中から字幕を出す。オフなら話し終えてから出し、途中の字幕の料金がかからない
    pub live_subtitles: bool,
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
    /// Chrome のモードの「ハイブリッドにする」の提案を閉じた
    pub hybrid_hint_closed: bool,
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
    /// 確定字幕の音声認識（OpenAI）
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

/// 音声認識と翻訳をどこで動かすか（DESIGN.md 9.6）
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum SpeechEngine {
    /// 途中の字幕は Chrome、確定は OpenAI の高い精度の結果に置き換える
    Hybrid,
    /// すべて OpenAI。Chrome が要らず、PC の負荷が小さい。
    /// 0.3 より前の設定ファイルはこの値になる（Chrome のウィンドウを黙って開かない）。
    /// 新しく使い始める人は Chrome から始める（`load`）
    #[default]
    OpenAi,
    /// 音声認識と翻訳は Chrome、読みと用語は中核。無料で、用語の判定も LLM を使わない。
    /// 0.3 の rc には `local` や `free` で保存した版がある
    #[serde(alias = "local", alias = "free")]
    Chrome,
}

/// 字幕の文字の出どころ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSource {
    Chrome,
    GptLiveTranscribe,
    GptTranscribe,
}

/// 翻訳の出どころ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranslationSource {
    ChromeTranslator,
    Llm,
}

/// ふりがなの出どころ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RubySource {
    Lindera,
    Llm,
}

/// 用語の強調の出どころ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmphasisSource {
    Terms,
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

/// モードと設定から決まる、字幕の作り方。README の「しくみ」の表を写したもの。
/// 呼び出し側はモードではなくこれを見る（DESIGN.md 9.6）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub partial: Phase,
    pub finals: Phase,
    /// 翻訳だけを見せる
    pub translation_only: bool,
}

impl Phase {
    /// PC の中で読みか用語を付けるか
    pub fn marks(&self) -> bool {
        self.ruby == Some(RubySource::Lindera) || self.emphasis == Some(EmphasisSource::Terms)
    }
}

impl Plan {
    /// Chrome を起動するか
    pub fn uses_chrome(&self) -> bool {
        self.partial.text == Some(TextSource::Chrome)
    }

    /// API キーかギフトコードが要るか
    pub fn needs_key(&self) -> bool {
        self.finals.text == Some(TextSource::GptTranscribe)
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
            speech_engine: SpeechEngine::default(),
            mic_language: "ja".into(),
            mic_device: None,
            translate_to: vec!["en".into()],
            deadlock_mode: true,
            live_subtitles: true,
            subtitle_display: SubtitleDisplay::default(),
            subtitle_style: SubtitleStyle::default(),
            item_cards: ItemCards::default(),
            rank_language: None,
            rank_content: RankContent::Gauge,
            rank_anchor: Anchor::TopLeft,
            obs_seen: false,
            onboarding_closed: false,
            hybrid_hint_closed: false,
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

    /// モードと設定から決まる、字幕の作り方
    pub fn plan(&self) -> Plan {
        use EmphasisSource as Emphasis;
        use RubySource as Ruby;
        use TextSource as Text;
        use TranslationSource as Translation;

        let translates = !self.translate_to.is_empty();
        let translation_only = translates && !self.subtitle_display.show_source;
        // 翻訳だけなら見えないので、作らない。形態素解析の辞書は日本語だけ
        let ruby = self.subtitle_display.furigana && !translation_only;
        let lindera = (ruby && self.mic_language == "ja").then_some(Ruby::Lindera);
        let terms = (self.deadlock_mode && self.mic_language == "ja").then_some(Emphasis::Terms);
        let chrome_translation = translates.then_some(Translation::ChromeTranslator);
        let llm = Phase {
            text: Some(Text::GptTranscribe),
            translation: translates.then_some(Translation::Llm),
            ruby: ruby.then_some(Ruby::Llm),
            emphasis: self.deadlock_mode.then_some(Emphasis::Llm),
        };
        let chrome = Phase {
            text: Some(Text::Chrome),
            translation: chrome_translation,
            ruby: lindera,
            emphasis: terms,
        };
        let (partial, finals) = match self.speech_engine {
            SpeechEngine::Hybrid => (chrome, llm),
            SpeechEngine::Chrome => (chrome, chrome),
            SpeechEngine::OpenAi => {
                // 翻訳だけなら途中の字幕は見えないので、料金をかけない
                let live = self.live_subtitles && !translation_only;
                let partial = Phase {
                    text: live.then_some(Text::GptLiveTranscribe),
                    translation: None,
                    ruby: lindera.filter(|_| live),
                    emphasis: terms.filter(|_| live),
                };
                (partial, llm)
            }
        };
        Plan {
            partial,
            finals,
            translation_only,
        }
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
        if let Some(engine) = patch.speech_engine {
            next.speech_engine = engine;
        }
        if let Some(enabled) = patch.live_subtitles {
            next.live_subtitles = enabled;
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
    #[ts(optional)]
    pub live_subtitles: Option<bool>,
    #[ts(optional)]
    pub speech_engine: Option<SpeechEngine>,
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
    keep_readable(&mut json, &mut Vec::new());
    serde_json::from_value(json)
}

pub fn load(path: &Path) -> std::io::Result<Settings> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Settings {
                speech_engine: SpeechEngine::Chrome,
                rank_language: Some("en".into()),
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
            speech_engine: SpeechEngine::OpenAi,
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

    #[test]
    fn only_the_openai_engine_uses_realtime() {
        let with = |speech_engine| Settings {
            speech_engine,
            ..Default::default()
        };
        assert_eq!(
            with(SpeechEngine::OpenAi).plan().partial.text,
            Some(TextSource::GptLiveTranscribe)
        );
        assert_eq!(
            with(SpeechEngine::Hybrid).plan().partial.text,
            Some(TextSource::Chrome)
        );
        assert_eq!(
            with(SpeechEngine::Chrome).plan().partial.text,
            Some(TextSource::Chrome)
        );
    }

    #[test]
    fn the_rc_names_of_the_chrome_engine_are_read() {
        for saved in ["chrome", "local", "free"] {
            let engine: SpeechEngine = serde_json::from_value(saved.into()).unwrap();
            assert_eq!(engine, SpeechEngine::Chrome);
        }
        assert_eq!(
            serde_json::to_value(SpeechEngine::Chrome).unwrap(),
            serde_json::json!("chrome")
        );
    }

    fn plan_of(speech_engine: SpeechEngine) -> Plan {
        Settings {
            speech_engine,
            translate_to: vec!["en".into()],
            deadlock_mode: true,
            live_subtitles: true,
            ..Default::default()
        }
        .plan()
    }

    /// README の「しくみ」の表と同じ値。表を変えたら、ここも変える
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
        let chrome = phase(T::Chrome, Some(Tr::ChromeTranslator), R::Lindera, E::Terms);
        let llm = phase(T::GptTranscribe, Some(Tr::Llm), R::Llm, E::Llm);
        let live = phase(T::GptLiveTranscribe, None, R::Lindera, E::Terms);

        let hybrid = plan_of(SpeechEngine::Hybrid);
        assert_eq!((hybrid.partial, hybrid.finals), (chrome, llm));
        let openai = plan_of(SpeechEngine::OpenAi);
        assert_eq!((openai.partial, openai.finals), (live, llm));
        let free = plan_of(SpeechEngine::Chrome);
        assert_eq!((free.partial, free.finals), (chrome, chrome));
    }

    #[test]
    fn chrome_and_keys_follow_the_plan() {
        assert!(plan_of(SpeechEngine::Hybrid).uses_chrome());
        assert!(plan_of(SpeechEngine::Hybrid).needs_key());
        assert!(plan_of(SpeechEngine::Chrome).uses_chrome());
        assert!(!plan_of(SpeechEngine::Chrome).needs_key());
        assert!(!plan_of(SpeechEngine::OpenAi).uses_chrome());
        assert!(plan_of(SpeechEngine::OpenAi).needs_key());
    }

    #[test]
    fn settings_turn_items_off_in_the_plan() {
        let with = |change: fn(&mut Settings)| {
            let mut settings = Settings {
                speech_engine: SpeechEngine::Chrome,
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
        assert_eq!(load(&path).unwrap().speech_engine, SpeechEngine::Chrome);
        // 前の版の設定ファイルには `speechEngine` がない
        std::fs::write(&path, r#"{ "micLanguage": "ja" }"#).unwrap();
        assert_eq!(load(&path).unwrap().speech_engine, SpeechEngine::OpenAi);
        // 選んだモードは保存される
        let hybrid = Settings {
            speech_engine: SpeechEngine::Hybrid,
            ..Default::default()
        };
        save(&path, &hybrid).unwrap();
        assert_eq!(load(&path).unwrap().speech_engine, SpeechEngine::Hybrid);
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
    fn live_subtitles_are_on_unless_turned_off() {
        let s = Settings::default();
        assert!(s.live_subtitles);
        let patch = SettingsPatch {
            live_subtitles: Some(false),
            ..Default::default()
        };
        assert!(!s.apply(patch).unwrap().live_subtitles);
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
        assert_eq!(s.speech_engine, SpeechEngine::Chrome);
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
