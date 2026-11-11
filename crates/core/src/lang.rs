//! 言語の対応表（DESIGN.md 6.2）。言語を増やすときに触るのはここだけ。

use crate::types::LangCode;

pub struct Language {
    /// 内部の言語コード（BCP 47）
    pub code: &'static str,
    /// deadlock-api の `language` パラメータ
    pub api: &'static str,
    /// 音声認識の `languages`（ISO 639-1）
    pub stt: &'static str,
    pub label: &'static str,
}

const fn lang(
    code: &'static str,
    api: &'static str,
    stt: &'static str,
    label: &'static str,
) -> Language {
    Language {
        code,
        api,
        stt,
        label,
    }
}

pub const LANGUAGES: &[Language] = &[
    lang("ja", "japanese", "ja", "日本語"),
    lang("en", "english", "en", "English"),
    lang("ru", "russian", "ru", "Русский"),
    lang("zh-Hans", "schinese", "zh", "简体中文"),
    lang("zh-Hant", "tchinese", "zh", "繁體中文"),
    lang("ko", "koreana", "ko", "한국어"),
    lang("th", "thai", "th", "ไทย"),
    lang("vi", "vietnamese", "vi", "Tiếng Việt"),
    lang("id", "indonesian", "id", "Bahasa Indonesia"),
    lang("de", "german", "de", "Deutsch"),
    lang("fr", "french", "fr", "Français"),
    lang("es", "spanish", "es", "Español"),
    lang("es-419", "latam", "es", "Español (Latinoamérica)"),
    lang("pt-PT", "portuguese", "pt", "Português"),
    lang("pt-BR", "brazilian", "pt", "Português (Brasil)"),
    lang("it", "italian", "it", "Italiano"),
    lang("nl", "dutch", "nl", "Nederlands"),
    lang("pl", "polish", "pl", "Polski"),
    lang("uk", "ukrainian", "uk", "Українська"),
    lang("cs", "czech", "cs", "Čeština"),
    lang("tr", "turkish", "tr", "Türkçe"),
    lang("el", "greek", "el", "Ελληνικά"),
    lang("hu", "hungarian", "hu", "Magyar"),
    lang("ro", "romanian", "ro", "Română"),
    lang("bg", "bulgarian", "bg", "Български"),
    lang("sv", "swedish", "sv", "Svenska"),
    lang("da", "danish", "da", "Dansk"),
    lang("no", "norwegian", "no", "Norsk"),
    lang("fi", "finnish", "fi", "Suomi"),
];

pub fn find(code: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.code == code)
}

/// 英語での言語の名前（翻訳専門のモデルへの指示文に書く）。deadlock-api の名前から作る
pub fn english_name(code: &str) -> String {
    let api = find(code).map_or(code, |l| l.api);
    match api {
        "schinese" => "Simplified Chinese".to_owned(),
        "tchinese" => "Traditional Chinese".to_owned(),
        "koreana" => "Korean".to_owned(),
        "latam" => "Latin American Spanish".to_owned(),
        "brazilian" => "Brazilian Portuguese".to_owned(),
        api => {
            let mut letters = api.chars();
            let first = letters.next().map(|c| c.to_ascii_uppercase());
            first.into_iter().chain(letters).collect()
        }
    }
}

/// 未知のコードを取り除き、重複をなくす（順序は保つ）
pub fn normalize(codes: impl IntoIterator<Item = LangCode>) -> Vec<LangCode> {
    let mut out: Vec<LangCode> = Vec::new();
    for code in codes {
        if find(&code).is_some() && !out.contains(&code) {
            out.push(code);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn covers_all_deadlock_api_languages_once() {
        let codes: HashSet<_> = LANGUAGES.iter().map(|l| l.code).collect();
        let apis: HashSet<_> = LANGUAGES.iter().map(|l| l.api).collect();
        assert_eq!(codes.len(), 29);
        assert_eq!(apis.len(), 29);
    }

    #[test]
    fn english_names_come_from_the_api_names() {
        assert_eq!(english_name("ru"), "Russian");
        assert_eq!(english_name("zh-Hant"), "Traditional Chinese");
        assert_eq!(english_name("pt-BR"), "Brazilian Portuguese");
    }

    #[test]
    fn normalize_drops_unknown_and_duplicates() {
        let got = normalize(["en", "xx", "ja", "en"].map(String::from));
        assert_eq!(got, vec!["en".to_owned(), "ja".to_owned()]);
    }
}
