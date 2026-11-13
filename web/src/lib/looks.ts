// 文字の見た目の見本（LookDemo.svelte）。値は操作パネルで選べるものだけにし、アプリで同じ見た目を作れるようにする。
// フォントはカタログ（crates/core/src/font-catalog.json）のもので、アプリと同じく言語ごとに選ぶ。日本語のフォントを
// 字幕の日本語の行とランクに、英語のフォントを訳文の行に当てる。見本の字だけを入れた woff2 は scripts/fonts.ts が作る
import type { Background } from "../../../frontend/src/generated/Background";
import type { FontStyle } from "../../../frontend/src/generated/FontStyle";
import type { TextLook } from "../../../frontend/src/generated/TextLook";

export type Look = {
  label: string;
  text: TextLook;
  background: Background;
  /** 日本語の行とランク */
  fontStyle: FontStyle;
  /** 英語の訳文の行 */
  english: FontStyle;
};

const TEXT: TextLook = {
  color: null,
  outline: { width: null, color: null },
  shadow: { enabled: null, blur: null, color: null },
};
const NO_BAND: Background = { enabled: false, color: null, opacity: null };
const FONT: FontStyle = { font: null, weight: null, letterSpacing: null };

export const LOOKS: Look[] = [
  { label: "標準", text: TEXT, background: NO_BAND, fontStyle: FONT, english: FONT },
  {
    label: "帯",
    text: TEXT,
    background: { ...NO_BAND, enabled: true },
    fontStyle: FONT,
    english: FONT,
  },
  {
    label: "ポップ",
    text: { ...TEXT, color: "#ffe14d", outline: { width: 0.24, color: "#2a1600" } },
    background: NO_BAND,
    fontStyle: { font: "Dela Gothic One", weight: 400, letterSpacing: null },
    english: { font: "Bangers", weight: 400, letterSpacing: 0.02 },
  },
  {
    label: "かわいい",
    text: { ...TEXT, outline: { width: 0.18, color: "#e8559a" } },
    background: NO_BAND,
    // 線の太い字にする（細い字は、色の縁取りに白が埋もれる）
    fontStyle: { font: "Mochiy Pop One", weight: 400, letterSpacing: 0.02 },
    english: { font: "Sniglet", weight: 800, letterSpacing: null },
  },
  {
    label: "ドット",
    text: { ...TEXT, outline: { width: 0.2, color: "#12306b" } },
    background: NO_BAND,
    fontStyle: { font: "DotGothic16", weight: 400, letterSpacing: 0.02 },
    english: { font: "VT323", weight: 400, letterSpacing: null },
  },
];

/** 最初に出す見た目。標準のままだと、変えられることが伝わらない */
export const FIRST_LOOK = LOOKS.findIndex((look) => look.label === "ポップ");

/** assets/fonts の中のファイル名 */
export const fontFile = (family: string, weight: number) =>
  `${family.toLowerCase().replaceAll(" ", "-")}-${weight}.woff2`;
