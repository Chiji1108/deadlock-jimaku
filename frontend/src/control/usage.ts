// 料金の見せ方（上の帯と「料金」タブで同じにする）
import type { UsageView } from "../generated/UsageView";

type Usage = UsageView["session"];

/** 1 ドル未満は 3 桁（丸めてから決める。0.9996 を $1.000 にしない） */
export const usd = (n: number) => {
  const rounded = Math.round(n * 1000) / 1000;
  return `$${rounded < 1 ? rounded.toFixed(3) : rounded.toFixed(2)}`;
};

/** 使い道ごとの料金の合計（単価の分からない分は入らない） */
export const total = (usage: Usage | undefined) =>
  Object.values(usage ?? {}).reduce((sum, a) => sum + (a?.usd ?? 0), 0);

/** 単価の分からないモデルを使った分がある（合計に入っていない） */
export const unpriced = (usage: Usage | undefined) =>
  Object.values(usage ?? {}).some((a) => a?.unpriced);
