// 言語ごとに選んだフォントを lang.css の --font-* に渡す
import type { FontGroup } from "../generated/FontGroup";

type Fonts = Partial<Record<FontGroup, string>>;

/** フォントを選ぶ区分（lang.css と同じ）と、操作パネルで字形を確かめる見本の文 */
export const FONT_SAMPLES = {
  latin: "My favorite is Haze. Мой любимый герой Хейз.",
  ja: "私の一番好きなキャラはヘイズです",
  "zh-Hans": "我最喜欢的角色是海兹",
  "zh-Hant": "我最喜歡的角色是海茲",
  ko: "제가 가장 좋아하는 캐릭터는 헤이즈입니다",
  th: "ตัวละครที่ฉันชอบที่สุดคือเฮซ",
} satisfies Record<FontGroup, string>;

/** フォントの一覧の行に添える、字形を比べるための短い見本 */
export const FONT_SWATCHES = {
  latin: "Aa",
  ja: "あア亜",
  "zh-Hans": "汉字",
  "zh-Hant": "漢字",
  ko: "한글",
  th: "ไทย",
} satisfies Record<FontGroup, string>;

const GROUPS = Object.keys(FONT_SAMPLES) as FontGroup[];

/** その言語の文字をどのフォントで描くか（lang.css の区分） */
export const fontGroup = (lang: string): FontGroup => GROUPS.find((g) => g === lang) ?? "latin";

/** 見本の文字に付ける lang */
export const groupLang = (group: FontGroup) => (group === "latin" ? "en" : group);

/** 制御文字はサーバーが拒否するので、エスケープするのは引用符と \ だけ */
export const cssString = (name: string) => `"${name.replace(/["\\]/g, "\\$&")}"`;

export function applyOverlayFonts(
  fonts: Fonts,
  style: Pick<CSSStyleDeclaration, "setProperty" | "removeProperty"> = document.documentElement
    .style,
) {
  for (const group of GROUPS) {
    const family = fonts[group];
    if (family) style.setProperty(`--font-${group}`, `${cssString(family)},`);
    else style.removeProperty(`--font-${group}`);
  }
}
