//! 設定（DESIGN.md 5 章）。保存と操作パネルからの変更は Phase 5 で入れる。

use crate::lang;
use crate::types::LangCode;

#[derive(Clone, Debug)]
pub struct Settings {
    /// 配信者が話す言語
    pub mic_language: LangCode,
    /// 翻訳先（最大 4 言語。mic_language は除く）
    pub translate_to: Vec<LangCode>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mic_language: "ja".into(),
            translate_to: vec!["en".into()],
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
