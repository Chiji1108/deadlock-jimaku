// 料金の見せ方（上の帯と「料金」タブで同じにする）
import type { Amount } from "../generated/Amount";
import type { Use } from "../generated/Use";

export const usd = (n: number) => `$${n < 1 ? n.toFixed(3) : n.toFixed(2)}`;

/** 使い道ごとの料金の合計（単価の分からない分は入らない） */
export const total = (usage: { [key in Use]?: Amount } | undefined) =>
  Object.values(usage ?? {}).reduce((sum, a) => sum + (a?.usd ?? 0), 0);
