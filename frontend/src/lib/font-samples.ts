import type { FontEntry } from "../generated/FontEntry";

/** おすすめのフォントの見本のファイル名 */
export const fontSampleName = (family: string) =>
  `${family.toLowerCase().replaceAll(" ", "-")}.svg`;

/** 整数の MB で足りないものだけ小数を出す */
export function formatSize(bytes: number) {
  const mb = bytes / 1_000_000;
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
}

const lower = (font: FontEntry) => font.family.toLowerCase();

/**
 * おすすめはカタログの順。ほかの言語のおすすめも、取得したものや PC にあるものは
 * 「この PC のフォント」から選べるので、そちらは混ぜて名前順にする
 */
export function fontSections(fonts: FontEntry[], language: string) {
  const isRecommended = (font: FontEntry) => font.languages?.includes(language) ?? false;
  return {
    recommended: fonts.filter(isRecommended),
    others: fonts
      .filter((font) => !isRecommended(font) && font.sizeBytes === undefined)
      .toSorted((a, b) => (lower(a) < lower(b) ? -1 : 1)),
  };
}
