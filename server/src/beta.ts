// ベータの間の無料の提供（DESIGN.md 17.2）。開発者のキーで、いちばん安い組み合わせ（Qwen と OpenRouter）を中継する。
// 使う人はアプリが自動でもらうコードで見分ける。コードはおすそ分けのコードと同じ表に置き、同じ決まり（1 日の上限、止める）で動く

import { and, eq, sql } from "drizzle-orm";
import { day, month, type Db } from "./db";
import { betaGrants, betaSpend, codes, codeUsage, issuers } from "./schema";
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
/** ベータ全体の 1 日の上限（ドル）。届いたら、その日はどのコードも中継しない（悪用されても、ここで止まる）。
 * 最後の 1 回は上限を少し越える（届く前の要求は通す）ので、越える幅は 1 回の見積もりまで */
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

/** コードを出した所を数えるための値。IPv6 は先頭の 64 ビット（1 台が持つ範囲）にまとめる */
export function network(ip: string): string {
  if (!ip.includes(":")) return ip;
  const [head = "", tail] = ip.toLowerCase().split("::");
  const groups = (part: string) => (part ? part.split(":") : []);
  const [before, after] = [groups(head), groups(tail ?? "")];
  const zeros = tail === undefined ? [] : Array<string>(Math.max(8 - before.length - after.length, 0)).fill("0");
  const all = [...before, ...zeros, ...after];
  return all
    .slice(0, 4)
    .map((group) => group.replace(/^0+(?=.)/, ""))
    .join(":");
}

/**
 * コードを 1 つ出す。出せなければ理由（`tooMany` は同じ所から出しすぎ、`full` は今日の分が終わり）。
 * `place` は出した所を数えるための値（IP アドレスと日付のハッシュ。日をまたいで同じ所だと分からない）。
 * 数は「足してから見る」ので、同時に頼まれても上限を越えて出さない。同じ所から頼みすぎても、その日の合計は減らさない
 */
export async function grantBetaCode(db: Db, place: string, now: number): Promise<{ code: string } | { error: string }> {
  const today = day(now);
  const counted = (where: string) =>
    db
      .insert(betaGrants)
      .values({ day: today, place: where, count: 1 })
      .onConflictDoUpdate({
        target: [betaGrants.day, betaGrants.place],
        set: { count: sql`${betaGrants.count} + 1` },
      })
      .returning({ count: betaGrants.count })
      .get();
  if ((await counted(place)).count > BETA_CODES_PER_IP) return { error: "tooMany" };
  if ((await counted(ALL_PLACES)).count > BETA_CODES_PER_DAY) return { error: "full" };
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
  ]);
  return { code };
}

/** その日に出したコードの合計を数える行 */
const ALL_PLACES = "*";

/**
 * 外に送る前に、かかりそうな額（`estimate`。多めの見積もり）を、そのコードの今日の分とベータ全体の今日の分に先に足す。
 * どちらかが上限に届いていたら足さずに false（中継しない）。「見てから足す」と、同時に送られた要求がどれも通って
 * しまうので、上限に届いていないことを条件にした 1 つの文で足す。応答のあとに `settleBeta` で実際の額に直す
 */
export async function reserveBeta(db: Db, codeId: string, estimate: number, now: number): Promise<boolean> {
  const today = day(now);
  const spentToday = sql`CASE WHEN ${codes.day} = ${today} THEN ${codes.dayUsd} ELSE 0 END`;
  const mine = await db
    .update(codes)
    .set({ dayUsd: sql`${spentToday} + ${estimate}`, day: today })
    .where(and(eq(codes.id, codeId), sql`${spentToday} < COALESCE(${codes.dailyLimitUsd}, ${BETA_CODE_DAILY_USD})`))
    .returning({ id: codes.id })
    .get();
  if (!mine) return false;
  const all = await db
    .insert(betaSpend)
    .values({ day: today, usd: estimate })
    .onConflictDoUpdate({
      target: betaSpend.day,
      set: { usd: sql`${betaSpend.usd} + ${estimate}` },
      setWhere: sql`${betaSpend.usd} < ${BETA_DAILY_USD}`,
    })
    .returning({ usd: betaSpend.usd })
    .get();
  if (all) return true;
  // 全体の上限に届いていた。コードに足した分を戻す
  await db
    .update(codes)
    .set({ dayUsd: sql`MAX(${codes.dayUsd} - ${estimate}, 0)` })
    .where(and(eq(codes.id, codeId), eq(codes.day, today)));
  return false;
}

/** 先に足した見積もりを、実際の額に直す。月の集計には実際の額を足す。日が変わっていたら、今日の分は直さない */
export async function settleBeta(db: Db, codeId: string, estimate: number, actual: number, now: number) {
  const today = day(now);
  const diff = actual - estimate;
  await db.batch([
    db
      .insert(codeUsage)
      .values({ codeId, month: month(now), usd: actual })
      .onConflictDoUpdate({
        target: [codeUsage.codeId, codeUsage.month],
        set: { usd: sql`${codeUsage.usd} + excluded.usd` },
      }),
    db
      .update(codes)
      .set({ dayUsd: sql`MAX(${codes.dayUsd} + ${diff}, 0)` })
      .where(and(eq(codes.id, codeId), eq(codes.day, today))),
    db
      .update(betaSpend)
      .set({ usd: sql`MAX(${betaSpend.usd} + ${diff}, 0)` })
      .where(eq(betaSpend.day, today)),
  ]);
}
