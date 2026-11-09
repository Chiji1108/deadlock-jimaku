// D1 の読み書き（表は schema.ts）

import { eq, sql, type SQL } from "drizzle-orm";
import { drizzle, type DrizzleD1Database } from "drizzle-orm/d1";
import type { Provider } from "./price";
import { codes, codeUsage, issuers } from "./schema";
import { normalizeCode, sha256 } from "./secrets";

/** 期限を過ぎても、始めていた配信はこの間だけ続けられる（期限は字幕を始めるときに確かめる） */
export const GRACE_MS = 24 * 60 * 60 * 1000;

/** コードの 1 日の上限（ドル）の既定。話し続ける最悪の約 $2.5/時間で 1 人が 24 時間使っても届かない値（DESIGN.md 17.1） */
export const DEFAULT_DAILY_LIMIT_USD = 60;

export type Db = DrizzleD1Database;

/** D1 を Drizzle で開く。列の名前は snake_case（schema.ts） */
export function open(d1: D1Database): Db {
  return drizzle(d1, { casing: "snake_case" });
}

const issuerColumns = {
  id: issuers.id,
  encryptedKey: issuers.encryptedKey,
  encryptedXaiKey: issuers.encryptedXaiKey,
};

export type Issuer = Pick<typeof issuers.$inferSelect, keyof typeof issuerColumns>;

const codeColumns = {
  id: codes.id,
  issuerId: codes.issuerId,
  expiresAt: codes.expiresAt,
  revoked: codes.revoked,
  dailyLimitUsd: codes.dailyLimitUsd,
  day: codes.day,
  dayUsd: codes.dayUsd,
  /** おすそ分けした人の暗号化したキー */
  encryptedKey: issuers.encryptedKey,
  /** おすそ分けした人の暗号化した xAI のキー。NULL は預けていない */
  encryptedXaiKey: issuers.encryptedXaiKey,
};

export type Code = Omit<typeof codes.$inferSelect, "codeHash" | "memo" | "createdAt"> &
  Pick<typeof issuers.$inferSelect, "encryptedKey" | "encryptedXaiKey">;

export async function issuerByToken(db: Db, token: string): Promise<Issuer | null> {
  const row = await db
    .select(issuerColumns)
    .from(issuers)
    .where(eq(issuers.adminTokenHash, await sha256(token)))
    .get();
  return row ?? null;
}

export async function codeByValue(db: Db, code: string): Promise<Code | null> {
  return codeWhere(db, eq(codes.codeHash, await sha256(normalizeCode(code))));
}

export async function codeById(db: Db, id: string): Promise<Code | null> {
  return codeWhere(db, eq(codes.id, id));
}

async function codeWhere(db: Db, where: SQL): Promise<Code | null> {
  const row = await db
    .select(codeColumns)
    .from(codes)
    .innerJoin(issuers, eq(issuers.id, codes.issuerId))
    .where(where)
    .get();
  return row ?? null;
}

/** 中継しない理由。期限は猶予（GRACE_MS）を足して見る。使えれば undefined */
export function refusal(code: Code, now: number): "revoked" | "expired" | "limit" | undefined {
  if (code.revoked) return "revoked";
  if (code.expiresAt !== null && now >= code.expiresAt + GRACE_MS) return "expired";
  if (overLimit(code, now)) return "limit";
  return undefined;
}

/** コードで中継する会社（OpenAI はいつも。xAI はキーを預けていれば） */
export function providers(code: Pick<Code, "encryptedXaiKey">): Provider[] {
  return code.encryptedXaiKey === null ? ["openai"] : ["openai", "xai"];
}

/** 実際に効く 1 日の上限 */
export function dailyLimit(code: Pick<Code, "dailyLimitUsd">): number {
  return code.dailyLimitUsd ?? DEFAULT_DAILY_LIMIT_USD;
}

/** 今日（UTC）使われた額。`dayUsd` が前の日のものなら 0 */
export function todayUsd(code: Pick<Code, "day" | "dayUsd">, now: number): number {
  return code.day === day(now) ? code.dayUsd : 0;
}

/** 今日（UTC）の上限に達したか */
export function overLimit(code: Code, now: number): boolean {
  return todayUsd(code, now) >= dailyLimit(code);
}

/** コードで使われた額を足す（月の集計と今日の分） */
export async function addUsage(db: Db, codeId: string, usd: number, now: number) {
  if (usd <= 0) return;
  const today = day(now);
  await db.batch([
    db
      .insert(codeUsage)
      .values({ codeId, month: month(now), usd })
      .onConflictDoUpdate({
        target: [codeUsage.codeId, codeUsage.month],
        set: { usd: sql`${codeUsage.usd} + excluded.usd` },
      }),
    db
      .update(codes)
      .set({
        dayUsd: sql`CASE WHEN ${codes.day} = ${today} THEN ${codes.dayUsd} + ${usd} ELSE ${usd} END`,
        day: today,
      })
      .where(eq(codes.id, codeId)),
  ]);
}

export function month(now: number): string {
  return new Date(now).toISOString().slice(0, 7);
}

export function day(now: number): string {
  return new Date(now).toISOString().slice(0, 10);
}
