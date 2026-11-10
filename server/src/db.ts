// D1 の読み書き（表は schema.ts）

import { eq, sql, type SQL } from "drizzle-orm";
import { drizzle, type DrizzleD1Database } from "drizzle-orm/d1";
import type { Provider } from "./price";
import { codes, codeUsage, issuerKeys, issuers } from "./schema";
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

export type Issuer = Pick<typeof issuers.$inferSelect, "id" | "tokenSalt">;

/** 会社の並び。OpenAI を先に置く */
export const PROVIDERS: readonly Provider[] = ["openai", "xai"];

const codeColumns = {
  id: codes.id,
  issuerId: codes.issuerId,
  expiresAt: codes.expiresAt,
  revoked: codes.revoked,
  dailyLimitUsd: codes.dailyLimitUsd,
  day: codes.day,
  dayUsd: codes.dayUsd,
};

export type Code = { [K in keyof typeof codeColumns]: (typeof codes.$inferSelect)[K] } & {
  /** おすそ分けした人が預けた、会社ごとの暗号化したキー */
  keys: Partial<Record<Provider, string>>;
};

export async function issuerByToken(db: Db, token: string): Promise<Issuer | null> {
  const row = await db
    .select({ id: issuers.id, tokenSalt: issuers.tokenSalt })
    .from(issuers)
    .where(eq(issuers.adminTokenHash, await sha256(token)))
    .get();
  return row ?? null;
}

/** 預けたキーの持ち主（キーのハッシュで探す） */
export async function issuerByKey(db: Db, keyHash: string): Promise<Issuer | null> {
  const row = await db
    .select({ id: issuers.id, tokenSalt: issuers.tokenSalt })
    .from(issuerKeys)
    .innerJoin(issuers, eq(issuers.id, issuerKeys.issuerId))
    .where(eq(issuerKeys.keyHash, keyHash))
    .get();
  return row ?? null;
}

/** 預けたキーの会社 */
export async function issuerProviders(db: Db, issuerId: string): Promise<Provider[]> {
  const rows = await db
    .select({ provider: issuerKeys.provider })
    .from(issuerKeys)
    .where(eq(issuerKeys.issuerId, issuerId));
  return PROVIDERS.filter((p) => rows.some((r) => r.provider === p));
}

export async function codeByValue(db: Db, code: string): Promise<Code | null> {
  return codeWhere(db, eq(codes.codeHash, await sha256(normalizeCode(code))));
}

export async function codeById(db: Db, id: string): Promise<Code | null> {
  return codeWhere(db, eq(codes.id, id));
}

/** コードと、預けたキー（会社ごとに 1 行になるので、1 つにまとめる） */
async function codeWhere(db: Db, where: SQL): Promise<Code | null> {
  const rows = await db
    .select({ code: codeColumns, provider: issuerKeys.provider, key: issuerKeys.encryptedKey })
    .from(codes)
    .leftJoin(issuerKeys, eq(issuerKeys.issuerId, codes.issuerId))
    .where(where);
  const first = rows[0];
  if (!first) return null;
  const keys: Code["keys"] = {};
  for (const { provider, key } of rows) if (provider && key) keys[provider] = key;
  return { ...first.code, keys };
}

/** 中継しない理由。期限は猶予（GRACE_MS）を足して見る。使えれば undefined */
export function refusal(code: Code, now: number): "revoked" | "expired" | "limit" | undefined {
  if (code.revoked) return "revoked";
  if (code.expiresAt !== null && now >= code.expiresAt + GRACE_MS) return "expired";
  if (overLimit(code, now)) return "limit";
  return undefined;
}

/** コードで中継する会社（おすそ分けした人がキーを預けた会社） */
export function providers(code: Pick<Code, "keys">): Provider[] {
  return PROVIDERS.filter((p) => code.keys[p] !== undefined);
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
