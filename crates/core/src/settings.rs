//! 設定（DESIGN.md 5 章）。保存と操作パネルからの変更は Phase 5 で入れる。

use crate::lang;
use crate::types::LangCode;

#[derive(Clone, Debug)]
pub struct Settings {
    /// 配信者が話す言語
    pub mic_language: LangCode,
    /// マイクのデバイス名。None なら OS の既定の入力デバイス
    pub mic_device: Option<String>,
    /// 翻訳先（最大 4 言語。mic_language は除く）
    pub translate_to: Vec<LangCode>,
    pub models: Models,
    /// Realtime の streaming delay（80〜1200、または 2400）
    pub target_streaming_delay_ms: u32,
    pub segmentation: Segmentation,
}

#[derive(Clone, Debug)]
pub struct Models {
    pub realtime: String,
    pub batch: String,
    /// 翻訳（OpenAI）。質を優先して選んだ（DESIGN.md 9.3）
    pub translate: String,
    /// 辞書の漢字名の読み（OpenAI）。名前ごとにキャッシュする
    pub readings: String,
    /// translate / readings の推論の深さ。推論しないモデルにするときは None
    pub reasoning_effort: Option<String>,
}

/// 字幕の区切り（DESIGN.md 3.1）
#[derive(Clone, Debug)]
pub struct Segmentation {
    /// この長さの無音で Segment を閉じる
    pub min_silence_ms: u32,
    /// この長さを超えたら、次の短い無音で閉じる
    pub max_segment_ms: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mic_language: "ja".into(),
            mic_device: None,
            translate_to: vec!["en".into()],
            models: Models {
                realtime: "voxtral-mini-transcribe-realtime-3".into(),
                batch: "voxtral-small-transcribe-3".into(),
                translate: "gpt-6.1-terra".into(),
                readings: "gpt-6.1-terra".into(),
                reasoning_effort: Some("none".into()),
            },
            target_streaming_delay_ms: 480,
            segmentation: Segmentation {
                min_silence_ms: 400,
                max_segment_ms: 8000,
            },
        }
    }
}

impl Settings {
    /// 辞書に名前を載せる言語（DESIGN.md 8.2）
    pub fn glossary_languages(&self) -> Vec<LangCode> {
        lang::normalize(
            [self.mic_language.clone(), "en".into()]
                .into_iter()
                .chain(self.translate_to.iter().cloned()),
        )
    }
}
