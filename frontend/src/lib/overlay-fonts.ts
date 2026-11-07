// 言語ごとに選んだフォントを lang.css の --font-* に渡す
import type { FontGroup } from "../generated/FontGroup";

type Fonts = Partial<Record<FontGroup, string>>;

const GROUPS: FontGroup[] = ["latin", "ja", "zh-Hans", "zh-Hant", "ko", "th"];

/** その言語の文字をどのフォントで描くか（lang.css の区分） */
export const fontGroup = (lang: string): FontGroup => GROUPS.find((g) => g === lang) ?? "latin";

/** 名前の形はサーバーが確かめているので、そのまま CSS の文字列にできる */
export function applyOverlayFonts(
  fonts: Fonts,
  style: Pick<CSSStyleDeclaration, "setProperty" | "removeProperty"> = document.documentElement
    .style,
) {
  for (const group of GROUPS) {
    const family = fonts[group];
    if (family) style.setProperty(`--font-${group}`, `"${family}",`);
    else style.removeProperty(`--font-${group}`);
  }
}
