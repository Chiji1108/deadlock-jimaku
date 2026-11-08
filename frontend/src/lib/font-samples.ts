import type { FontEntry } from "../generated/FontEntry";

/** 空文字列のフォントの名前。lang.css のまま */
export const STANDARD = "標準";

/** Google Fonts のフォントの見本のファイル名 */
export const fontSampleName = (family: string) =>
  `${family.toLowerCase().replaceAll(" ", "-")}.svg`;

/** 整数の MB で足りないものだけ小数を出す */
export function formatSize(bytes: number) {
  const mb = bytes / 1_000_000;
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
}

const lower = (font: FontEntry) => font.family.toLowerCase();

/**
 * Google Fonts はカタログの順。ほかの言語のものも、取得したものや PC にあるものは
 * 「この PC のフォント」から選べるので、そちらは混ぜて名前順にする
 */
export function fontSections(fonts: FontEntry[], language: string) {
  const forLanguage = (font: FontEntry) => font.languages?.includes(language) ?? false;
  return {
    googleFonts: fonts.filter(forLanguage),
    others: fonts
      .filter((font) => !forLanguage(font) && font.sizeBytes === undefined)
      .toSorted((a, b) => (lower(a) < lower(b) ? -1 : 1)),
  };
}

/**
 * 矢印キーで次に移るカードの番号。-1 は検索欄（at も検索欄なら -1）、undefined は動かない。
 * 見出しで段がずれ、最後の段は埋まらないので、列は lefts（カードの左端）で見分ける
 */
export function cardTarget(lefts: number[], at: number, key: string) {
  if (at < 0) return key === "ArrowDown" && lefts.length > 0 ? 0 : undefined;
  const column = lefts.flatMap((left, i) => (left === lefts[at] ? [i] : []));
  const row = column.indexOf(at);
  if (key === "ArrowLeft") return at > 0 ? at - 1 : undefined;
  if (key === "ArrowRight") return at + 1 < lefts.length ? at + 1 : undefined;
  if (key === "ArrowUp") return row > 0 ? column[row - 1] : -1;
  return key === "ArrowDown" ? column[row + 1] : undefined;
}
