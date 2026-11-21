// 翻訳で使う言語（アプリの crates/core/src/lang.rs と同じ並び）。言語を増やすときは、両方に足す

/** 言語コード（BCP 47）ごとの、英語での名前（指示文に書く）と、Qwen-MT の言語コード（地域は分けない。中国語だけ別） */
const LANGUAGES: Record<string, { name: string; qwen: string }> = {
  ja: { name: "Japanese", qwen: "ja" },
  en: { name: "English", qwen: "en" },
  ru: { name: "Russian", qwen: "ru" },
  "zh-Hans": { name: "Simplified Chinese", qwen: "zh" },
  "zh-Hant": { name: "Traditional Chinese", qwen: "zh_tw" },
  ko: { name: "Korean", qwen: "ko" },
  th: { name: "Thai", qwen: "th" },
  vi: { name: "Vietnamese", qwen: "vi" },
  id: { name: "Indonesian", qwen: "id" },
  de: { name: "German", qwen: "de" },
  fr: { name: "French", qwen: "fr" },
  es: { name: "Spanish", qwen: "es" },
  "es-419": { name: "Latin American Spanish", qwen: "es" },
  "pt-PT": { name: "Portuguese", qwen: "pt" },
  "pt-BR": { name: "Brazilian Portuguese", qwen: "pt" },
  it: { name: "Italian", qwen: "it" },
  nl: { name: "Dutch", qwen: "nl" },
  pl: { name: "Polish", qwen: "pl" },
  uk: { name: "Ukrainian", qwen: "uk" },
  cs: { name: "Czech", qwen: "cs" },
  tr: { name: "Turkish", qwen: "tr" },
  el: { name: "Greek", qwen: "el" },
  hu: { name: "Hungarian", qwen: "hu" },
  ro: { name: "Romanian", qwen: "ro" },
  bg: { name: "Bulgarian", qwen: "bg" },
  sv: { name: "Swedish", qwen: "sv" },
  da: { name: "Danish", qwen: "da" },
  no: { name: "Norwegian", qwen: "nb" },
  fi: { name: "Finnish", qwen: "fi" },
};

/** 知っている言語なら、その名前とコード */
export const language = (code: unknown) =>
  typeof code === "string" && Object.hasOwn(LANGUAGES, code) ? LANGUAGES[code] : undefined;
