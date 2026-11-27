// 買ったアイテムに添える統計の見せ方（DESIGN.md 18.4）

/** 選んだ言語のラベル。用意のない言語は英語で出す */
const LABELS = {
  ja: { share: "購入率", boughtAt: "平均の購入", rare: "珍しい", staple: "定番" },
  en: { share: "Pick rate", boughtAt: "Avg. buy time", rare: "Rare", staple: "Staple" },
};

export const labels = (lang: string) => (lang === "ja" ? LABELS.ja : LABELS.en);

/** めったに買われないアイテムと、ほとんどの試合で買われるアイテムの線（そのヒーローの試合のうちの割合） */
const RARE = 0.02;
const STAPLE = 0.5;

export function badge(share: number): "rare" | "staple" | undefined {
  if (share < RARE) return "rare";
  if (share >= STAPLE) return "staple";
  return undefined;
}

/** 10% より小さい割合は、小数 1 桁まで出す（0.4% と 4% を見分ける） */
export function percent(share: number): string {
  const value = share * 100;
  return `${value < 10 ? Math.round(value * 10) / 10 : Math.round(value)}%`;
}

/** 試合が始まってからの分と秒 */
export function clock(seconds: number): string {
  const total = Math.round(seconds);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}
