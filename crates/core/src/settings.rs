//! 設定（DESIGN.md 5 章）。`{データディレクトリ}/settings.json` に保存し、操作パネルから変える。
//!
//! API キーはここには入れない（`secrets`。OS の資格情報ストアに保存する）。

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::lang;
use crate::types::LangCode;

/// `{データディレクトリ}` の中の設定ファイル
pub const FILE: &str = "settings.json";

/// 翻訳先の上限（多いほど出力が増えて遅くなる）
pub const MAX_TRANSLATE_TO: usize = 4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Settings {
    /// マイクを字幕にするか（操作パネルの開始・停止）
    pub transcription_enabled: bool,
    /// 配信者が話す言語
    pub mic_language: LangCode,
    /// マイクのデバイス名。None なら OS の既定の入力デバイス
    #[ts(optional)]
    pub mic_device: Option<String>,
    /// 翻訳先（最大 4 言語。mic_language は除く）
    pub translate_to: Vec<LangCode>,
    /// 字幕オーバーレイの見せ方。変えると表示中のオーバーレイにそのまま届く
    pub subtitle_display: SubtitleDisplay,
    /// ランク名の言語。None なら話す言語
    #[ts(optional)]
    pub rank_language: Option<LangCode>,
    // 以下の詳細項目は操作パネルで変えない（`ADVANCED`）
    pub models: Models,
    /// Realtime の streaming delay（80〜1200、または 2400）
    pub target_streaming_delay_ms: u32,
    pub segmentation: Segmentation,
    /// ランクを表示するアカウント（Steam の 32bit アカウント ID）。None ならランクを取りに行かない
    #[ts(optional)]
    pub rank_account: Option<u32>,
    /// ランクを取りに行く間隔（試合後すぐには反映されないので短くしない）
    #[ts(type = "number")]
    pub rank_poll_seconds: u64,
}

/// 既定値から変えたときだけ保存する項目（保存してしまうと、アプリの既定値を直しても古い値が残る）
const ADVANCED: [&str; 4] = [
    "models",
    "targetStreamingDelayMs",
    "segmentation",
    "rankPollSeconds",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct Models {
    pub realtime: String,
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
    /// 元の文（話した言葉）を出す
    pub show_source: bool,
    /// 元の文にふりがなを付ける
    pub furigana: bool,
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
            max_lines: 3,
            hold_seconds: 6.0,
        }
    }
}

impl SubtitleDisplay {
    pub const MAX_LINES: std::ops::RangeInclusive<u32> = 1..=5;
    pub const HOLD_SECONDS: std::ops::RangeInclusive<f32> = 2.0..=30.0;
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
            mic_language: "ja".into(),
            mic_device: None,
            translate_to: vec!["en".into()],
            subtitle_display: SubtitleDisplay::default(),
            rank_language: None,
            models: Models::default(),
            target_streaming_delay_ms: 480,
            segmentation: Segmentation::default(),
            rank_account: None,
            rank_poll_seconds: 180,
        }
    }
}

impl Default for Models {
    fn default() -> Self {
        Self {
            realtime: "voxtral-mini-transcribe-realtime-3".into(),
            batch: "voxtral-small-transcribe-3".into(),
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
            min_silence_ms: 400,
            max_segment_ms: 8000,
        }
    }
}

impl Settings {
    /// 辞書に名前を載せる言語（DESIGN.md 8.2）
    /// ランク名に使う言語
    pub fn rank_language(&self) -> LangCode {
        self.rank_language
            .clone()
            .unwrap_or_else(|| self.mic_language.clone())
    }

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
        self
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
            if !SubtitleDisplay::MAX_LINES.contains(&display.max_lines)
                || !SubtitleDisplay::HOLD_SECONDS.contains(&display.hold_seconds)
            {
                return Err("subtitleDisplay is out of range".into());
            }
            next.subtitle_display = display;
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
    /// SteamID64 か 32bit のアカウント ID。空文字列ならランクを表示しない
    #[ts(optional)]
    pub rank_account: Option<String>,
    #[ts(optional)]
    pub subtitle_display: Option<SubtitleDisplay>,
    /// 何 ms 黙ったら区切るか
    #[ts(optional)]
    pub min_silence_ms: Option<u32>,
    /// 空文字列なら話す言語
    #[ts(optional)]
    pub rank_language: Option<LangCode>,
}

/// ファイルがなければ既定値。壊れていたら脇に退けて既定値から始める。
/// 読めない（ほかのアプリが掴んでいるなど）ときは Err: 既定値で上書き保存しないため
pub fn load(path: &Path) -> std::io::Result<Settings> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(e),
    };
    match serde_json::from_slice::<Settings>(&bytes) {
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
        let display = SubtitleDisplay {
            max_lines: 2,
            ..SubtitleDisplay::default()
        };
        let next = s
            .apply(SettingsPatch {
                subtitle_display: Some(display.clone()),
                min_silence_ms: Some(700),
                rank_language: Some("en".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(next.subtitle_display, display);
        assert_eq!(next.segmentation.min_silence_ms, 700);
        assert_eq!(next.rank_language(), "en");

        let reset = next
            .apply(SettingsPatch {
                rank_language: Some(String::new()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(reset.rank_language(), "ja");

        for patch in [
            SettingsPatch {
                min_silence_ms: Some(50),
                ..Default::default()
            },
            SettingsPatch {
                subtitle_display: Some(SubtitleDisplay {
                    max_lines: 0,
                    ..SubtitleDisplay::default()
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
