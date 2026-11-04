// LP の見本のアイテム説明。アプリと同じ deadlock-ui のカードを描く。
// データはビルドのときに取ってページに埋める（訪問者ごとに全アイテムを読まない。カードの言語も揃う）
import type { Item } from "@deadlock-api/ui-core";
import { LINES } from "./demo";

type Linked = { name: string; image?: string };
export type Card = { item: Item; components: Linked[]; parents: Linked[] };
export type Cards = Record<string, Card>;

export async function fetchCards(): Promise<Cards> {
  const res = await fetch("https://api.deadlock-api.com/v1/assets/items?language=japanese");
  if (!res.ok) throw new Error(`failed to fetch items: ${res.status}`);
  const items = (await res.json()) as Item[];
  const linked = (item: Item): Linked => ({
    name: item.name,
    image: item.shop_image_webp ?? item.image_webp ?? undefined,
  });
  const card = (item: Item): Card => ({
    item,
    components: items.filter((i) => item.component_items?.includes(i.class_name)).map(linked),
    parents: items.filter((i) => i.component_items?.includes(item.class_name)).map(linked),
  });
  const names = new Set(LINES.map((line) => line.card));
  return Object.fromEntries(
    items.filter((i) => names.has(i.class_name)).map((i) => [i.class_name, card(i)]),
  );
}

let defined: Promise<void> | undefined;

/** カードの部品は重いので、ページが出てから読み込む */
export function defineCards() {
  defined ??= Promise.all([
    import("@deadlock-api/ui-core/dist/components/dl-provider.js"),
    import("@deadlock-api/ui-core/dist/components/dl-item-tooltip.js"),
  ]).then(([provider, tooltip]) => {
    provider.defineCustomElement();
    tooltip.defineCustomElement();
  });
  return defined;
}
