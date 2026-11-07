/** おすすめのフォントの見本のファイル名 */
export const fontSampleName = (family: string) =>
  `${family.toLowerCase().replaceAll(" ", "-")}.svg`;

/** 整数の MB で足りないものだけ小数を出す */
export function formatSize(bytes: number) {
  const mb = bytes / 1_000_000;
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
}
