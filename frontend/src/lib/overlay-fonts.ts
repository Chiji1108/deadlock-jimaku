// 言語ごとに選んだフォントを lang.css の --font に渡す
import type { Settings } from "../generated/Settings";

type Fonts = Settings["overlayFonts"];

/** 制御文字はサーバーが拒否するので、エスケープするのは引用符と \ だけ */
export const cssString = (name: string) => `"${name.replace(/["\\]/g, "\\$&")}"`;

/** その言語での言語名（한국어、Русский）。読めないコードならコードのまま */
export function endonym(lang: string) {
  try {
    return new Intl.DisplayNames([lang], { type: "language" }).of(lang) ?? lang;
  } catch {
    return lang;
  }
}

/** フォントの見本。日本語は言語名だと漢字だけになるので、かなも混ぜる */
export const fontSample = (lang: string) => (lang === "ja" ? "あいアイ漢字" : endonym(lang));

/**
 * 値の末尾のカンマで lang.css の標準の候補につなぐ。:lang(es) は es-419 にも一致するので
 * 属性の完全一致で言語を分け、[lang] を重ねて lang.css のリセットより詳細度を上げる
 */
export const fontRules = (fonts: Fonts) =>
  Object.entries(fonts)
    .map(([lang, family]) => `[lang][lang=${cssString(lang)}] { --font: ${cssString(family)}, }`)
    .join("\n");

let sheet: HTMLStyleElement | undefined;

export function applyOverlayFonts(fonts: Fonts) {
  sheet ??= document.head.appendChild(document.createElement("style"));
  sheet.textContent = fontRules(fonts);
}
