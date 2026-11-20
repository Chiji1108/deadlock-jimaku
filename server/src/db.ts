// D1 の読み書き（表は schema.ts）

import { and, eq, sql } from "drizzle-orm";
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

/** `User-Agent` の `deadlock-jimaku/0.9.7 (windows)` から、アプリの版。ほかの形なら null（0.9.6 までは伝えない） */
export function appVersion(agent: string | undefined): string | null {
  return agent?.match(/^deadlock-jimaku\/(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?) /)?.[1] ?? null;
}

/** コードを確かめに来た（字幕を始めた）日と、アプリの版を覚える。同じ日に同じ版なら書かない */
export async function seen(db: Db, id: string, version: string | null, now: number) {
  const today = day(now);
  await db
    .update(codes)
    .set({ seenDay: today, appVersion: version })
    .where(
      and(
        eq(codes.id, id),
        sql`(${codes.seenDay} IS NOT ${today} OR ${codes.appVersion} IS NOT ${version})`,
      ),
    );
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
