// 操作パネルで決めた文字の見た目を、オーバーレイの CSS に渡す。値がなければ何も書かず、
// Captions.svelte と RankBadge.svelte の既定の見た目と OBS のカスタム CSS の変数をそのまま使う
import type { FontStyle } from "../generated/FontStyle";
import type { TextLook } from "../generated/TextLook";

/** 書いた項目だけを変え、null は標準に戻す（JSON Merge Patch） */
export type MergePatch<T> = T extends object ? { [K in keyof T]?: MergePatch<T[K]> | null } : T;

/** 全部の項目を標準に戻す */
export const cleared = <T extends object>(style: T) =>
  Object.fromEntries(Object.keys(style).map((key) => [key, null])) as MergePatch<T>;

/** 標準の見た目の値。操作パネルが今の値として出す（描くのは CSS の既定） */
const COMMON = {
  color: "#ffffff",
  outlineColor: "#000000",
  shadowColor: "#000000",
  bandColor: "#000000",
  bandOpacity: 0.62,
  letterSpacing: 0,
};
/** 帯があるときの縁取り */
const BAND_OUTLINE = 0.08;
export const SUBTITLE_TEXT = { ...COMMON, outlineWidth: 0.18, shadowBlur: 0.15, weight: 700 };
export const RANK_TEXT = { ...COMMON, outlineWidth: 0.16, shadowBlur: 0.375, weight: 800 };
export const LINE_HEIGHT = 1.5;
export type TextStandard = typeof SUBTITLE_TEXT;

/** 帯があれば標準では影を付けない */
export const shadowOn = (look: TextLook) => look.shadow.enabled ?? !look.background.enabled;

export const outlineStandard = (look: TextLook, standard: TextStandard) =>
  look.background.enabled ? BAND_OUTLINE : standard.outlineWidth;

const declarations = (entries: [string, string | number | undefined | null][]) =>
  entries
    .filter(([, value]) => value !== undefined && value !== null)
    .map(([name, value]) => `${name}: ${value}`)
    .join("; ") || undefined;

const alpha = (opacity: number) =>
  Math.round(opacity * 255)
    .toString(16)
    .padStart(2, "0");

/** 確定前の文字は、選んだ色を落として見せる（標準の #c8c8c8 と同じくらいの比） */
const PENDING_ALPHA = alpha(0.78);

/**
 * オーバーレイに渡す CSS 変数。em の値は `unit` を掛ける。em を置くのは var() を使う要素なので、
 * 行ごとの文字の大きさに合う。ランクは標準の縁取りが根の要素の文字の大きさなので、そこに合わせる
 */
export function textVars(look: TextLook, unit = "1em") {
  const { color, outline, shadow, background } = look;
  const on = shadowOn(look);
  const width = outline.width ?? (background.enabled ? BAND_OUTLINE : undefined);
  const edge = (outline.width ?? 0) / 2 + (on ? (shadow.blur ?? 0) : 0);
  const custom = background.color !== null || background.opacity !== null;
  const scaled = (value: number | null | undefined) =>
    value === null || value === undefined ? undefined : `calc(${value} * ${unit})`;
  return declarations([
    ["--text-color", color],
    ["--pending-color", color && `${color}${PENDING_ALPHA}`],
    ["--outline-color", outline.color],
    ["--outline-width", scaled(width)],
    ["--text-shadow", on ? undefined : "none"],
    ["--shadow-blur", on ? scaled(shadow.blur) : undefined],
    ["--shadow-color", on ? shadow.color : undefined],
    [
      "--band-color",
      custom
        ? `${background.color ?? COMMON.bandColor}${alpha(background.opacity ?? COMMON.bandOpacity)}`
        : undefined,
    ],
    ["--edge", edge > 0 ? scaled(edge) : undefined],
  ]);
}

/** フォントに付く項目。p に直に書く（OBS のカスタム CSS より操作パネルの値を優先する） */
export const fontCss = (font?: FontStyle) =>
  font &&
  declarations([
    ["font-weight", font.weight],
    ["letter-spacing", font.letterSpacing === null ? undefined : `${font.letterSpacing}em`],
  ]);

/** フォントの面が分からないとき（標準のフォント）に並べる太さ */
const COMMON_WEIGHTS = [400, 500, 700, 800, 900];

/** 太さの選択肢。フォントの持つ面に、標準の太さと選んでいる太さを足す（なければ太字を合成する） */
export const weightChoices = (
  faces: number[] | undefined,
  standard: number,
  chosen?: number | null,
) =>
  [
    ...new Set([
      ...(faces?.length ? faces : COMMON_WEIGHTS),
      standard,
      ...(chosen ? [chosen] : []),
    ]),
  ].toSorted((a, b) => a - b);
