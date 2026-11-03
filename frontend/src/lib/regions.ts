import type { Region } from "../generated/Region";

/** リーダーボードの地域の名前（操作パネルとランクのオーバーレイで使う） */
export const REGIONS: Record<Region, { ja: string; en: string }> = {
  Asia: { ja: "アジア", en: "Asia" },
  Europe: { ja: "ヨーロッパ", en: "Europe" },
  NAmerica: { ja: "北米", en: "N. America" },
  SAmerica: { ja: "南米", en: "S. America" },
  Oceania: { ja: "オセアニア", en: "Oceania" },
};
