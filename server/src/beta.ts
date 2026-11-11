// ベータの間の無料の提供（DESIGN.md 17.2）。開発者のキーで、いちばん安い組み合わせ（Qwen と OpenRouter）を中継する。
// 使う人はアプリが自動でもらうコードで見分ける。コードはおすそ分けのコードと同じ表に置き、同じ決まり（1 日の上限、止める）で動く

import { and, eq, sql } from "drizzle-orm";
import { day, type Db } from "./db";
import { betaGrants, codes, issuers } from "./schema";
import { newCode, newId, normalizeCode, sha256 } from "./secrets";

/** ベータのコードの持ち主（開発者）。キーは表に置かず、Worker の秘密の設定から読む */
export const BETA_ISSUER = "beta";

/** ベータで中継する会社 */
export type BetaProvider = "qwen" | "openrouter";

export interface BetaEnv {
  /** 開発者のキー。どちらも入れなければ、ベータは閉じている（`wrangler secret put`） */
  BETA_QWEN_KEY?: string;
  BETA_OPENROUTER_KEY?: string;
}

/** 1 つのコード（1 台）の 1 日の上限（ドル）。いちばん安い組み合わせで約 14 時間分。ふつうの配信では届かない */
export const BETA_CODE_DAILY_USD = 0.5;
/** ベータ全体の 1 日の上限（ドル）。届いたら、その日はどのコードも中継しない（悪用されても、ここで止まる） */
export const BETA_DAILY_USD = 5;
/** 同じ所（IP アドレス）に 1 日に出すコードの数 */
export const BETA_CODES_PER_IP = 3;
/** 1 日に出すコードの数 */
export const BETA_CODES_PER_DAY = 100;

export function betaKey(env: BetaEnv, provider: BetaProvider): string | undefined {
  return provider === "qwen" ? env.BETA_QWEN_KEY : env.BETA_OPENROUTER_KEY;
}

/** 開発者がキーを入れている会社 */
export function betaProviders(env: BetaEnv): BetaProvider[] {
  return (["qwen", "openrouter"] as const).filter((p) => betaKey(env, p));
}

/** ベータの全部のコードが今日（UTC）使った額 */
export async function betaSpentToday(db: Db, now: number): Promise<number> {
  const row = await db
    .select({ usd: sql<number>`COALESCE(SUM(${codes.dayUsd}), 0)` })
    .from(codes)
    .where(and(eq(codes.issuerId, BETA_ISSUER), eq(codes.day, day(now))))
    .get();
  return row?.usd ?? 0;
}

/**
 * コードを 1 つ出す。出せなければ理由（`tooMany` は同じ所から出しすぎ、`full` は今日の分が終わり）。
 * `place` は出した所を数えるための値（IP アドレスと日付のハッシュ。日をまたいで同じ所だと分からない）
 */
export async function grantBetaCode(db: Db, place: string, now: number): Promise<{ code: string } | { error: string }> {
  const today = day(now);
  const granted = await db
    .select({
      here: sql<number>`COALESCE(SUM(CASE WHEN ${betaGrants.place} = ${place} THEN ${betaGrants.count} END), 0)`,
      all: sql<number>`COALESCE(SUM(${betaGrants.count}), 0)`,
    })
    .from(betaGrants)
    .where(eq(betaGrants.day, today))
    .get();
  if ((granted?.here ?? 0) >= BETA_CODES_PER_IP) return { error: "tooMany" };
  if ((granted?.all ?? 0) >= BETA_CODES_PER_DAY) return { error: "full" };
  const code = newCode();
  await db.batch([
    // 持ち主の行は、初めてのコードを出すときに作る（合言葉は使えない値。管理の口には入れない）
    db
      .insert(issuers)
      .values({ id: BETA_ISSUER, adminTokenHash: BETA_ISSUER, tokenSalt: "", createdAt: now })
      .onConflictDoNothing(),
    db.insert(codes).values({
      id: newId(),
      codeHash: await sha256(normalizeCode(code)),
      issuerId: BETA_ISSUER,
      memo: "beta",
      dailyLimitUsd: BETA_CODE_DAILY_USD,
      createdAt: now,
    }),
    db
      .insert(betaGrants)
      .values({ place, day: today, count: 1 })
      .onConflictDoUpdate({
        target: [betaGrants.place, betaGrants.day],
        set: { count: sql`${betaGrants.count} + 1` },
      }),
  ]);
  return { code };
}
