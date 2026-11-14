// D1 の読み書き（表は schema.ts）

import { eq } from "drizzle-orm";
import { drizzle, type DrizzleD1Database } from "drizzle-orm/d1";
import { codes } from "./schema";
import { normalizeCode, sha256 } from "./secrets";

/** 1 つのコード（1 台）の 1 日の上限（ドル）の既定。いちばん安い組み合わせで約 14 時間分。ふつうの配信では届かない */
export const CODE_DAILY_USD = 0.5;

export type Db = DrizzleD1Database;

/** D1 を Drizzle で開く。列の名前は snake_case（schema.ts） */
export function open(d1: D1Database): Db {
  return drizzle(d1, { casing: "snake_case" });
}

export type Code = Pick<typeof codes.$inferSelect, "id" | "revoked" | "dailyLimitUsd" | "day" | "dayUsd">;

const codeColumns = {
  id: codes.id,
  revoked: codes.revoked,
  dailyLimitUsd: codes.dailyLimitUsd,
  day: codes.day,
  dayUsd: codes.dayUsd,
};

export async function codeByValue(db: Db, code: string): Promise<Code | null> {
  const hash = await sha256(normalizeCode(code));
  return (await db.select(codeColumns).from(codes).where(eq(codes.codeHash, hash)).get()) ?? null;
}

export async function codeById(db: Db, id: string): Promise<Code | null> {
  return (await db.select(codeColumns).from(codes).where(eq(codes.id, id)).get()) ?? null;
}

/** 中継しない理由。使えれば undefined */
export function refusal(code: Code, now: number): "revoked" | "limit" | undefined {
  if (code.revoked) return "revoked";
  if (todayUsd(code, now) >= (code.dailyLimitUsd ?? CODE_DAILY_USD)) return "limit";
  return undefined;
}

/** 今日（UTC）使われた額。`dayUsd` が前の日のものなら 0 */
function todayUsd(code: Pick<Code, "day" | "dayUsd">, now: number): number {
  return code.day === day(now) ? code.dayUsd : 0;
}

export function month(now: number): string {
  return new Date(now).toISOString().slice(0, 7);
}

export function day(now: number): string {
  return new Date(now).toISOString().slice(0, 10);
}
