// 買ったアイテムの見本。操作パネルのプレビューと、OBS に出すテスト表示が、同じものを使う
import type { Purchase } from "../generated/Purchase";
import { glossary } from "./glossary.svelte";

/**
 * 実際の試合で出た組み合わせと数字（ミナ、時計 5:59。2026-11 の集計）。
 * 1 つ目は平均よりかなり早く、2 つ目はほとんどの試合で買われて、平均よりかなり遅い
 */
const SAMPLE: Purchase = {
  hero: "hero_vampirebat",
  clockSeconds: 359,
  items: [
    { item: "upgrade_blitz_bullets", rate: { share: 0.404, boughtAtSeconds: 996 } },
    { item: "upgrade_improved_spirit", rate: { share: 0.968, boughtAtSeconds: 149 } },
  ],
};

/** 見本の買い物。辞書を読む前は undefined（ゲームの画像は配布物に入れず、取ってきたものを使う） */
export function samplePurchase(): Purchase | undefined {
  if (!glossary.ready) return undefined;
  // ゲームの更新でヒーローやアイテムが無くなっていたら、辞書の初めのもので代える（数字は見本のまま）
  const hero = glossary.named("hero", SAMPLE.hero!) ?? glossary.heroes[0]?.[1];
  const items = SAMPLE.items.map(({ item, rate }, i) => ({
    item: (glossary.named("item", item) ?? glossary.items[i])?.className ?? item,
    rate,
  }));
  return { ...SAMPLE, hero: hero?.className ?? null, items };
}
