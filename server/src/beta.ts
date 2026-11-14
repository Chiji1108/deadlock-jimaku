// ベータ期間中の無料の提供（DESIGN.md 17.2）。開発者のキーで、いちばん安い組み合わせ（Qwen と OpenRouter）を中継する。
// 使う人はアプリが自動でもらうコードで見分ける。コードには 1 日の上限があり、管理画面から止められる

import { and, eq, inArray, sql } from "drizzle-orm";
import { CODE_DAILY_USD, day, month, type Db } from "./db";
import { betaGrants, betaKeys, betaSettings, betaSpend, codes, codeUsage } from "./schema";
import { decrypt, newCode, newId, normalizeCode, sha256 } from "./secrets";

/** ベータで中継する会社 */
export type BetaProvider = "qwen" | "openrouter";

export interface BetaEnv {
  /** 開発者のキー（管理画面ができる前の入れ方。`wrangler secret put`）。管理画面で足したキーがあれば、そちらを先に使う */
  BETA_QWEN_KEY?: string;
  BETA_OPENROUTER_KEY?: string;
}

/** ベータ全体の 1 日の上限（ドル）の既定。届いたら、その日はどのコードも中継しない（悪用されても、ここで止まる）。
 * 最後の 1 回は上限を少し越える（届く前の要求は通す）ので、越える幅は 1 回の見積もりまで */
export const BETA_DAILY_USD = 5;
/** 管理画面から入れられる上限のいちばん大きい値（ドル）。桁を打ち間違えても、ここで止まる */
export const BETA_LIMIT_MAX_USD = 1000;
/** 同じ所（IP アドレス）に 1 日に出すコードの数 */
export const BETA_CODES_PER_IP = 3;
/** 1 日に出すコードの数 */
export const BETA_CODES_PER_DAY = 100;

/** 外に送るときに使う開発者のキー。`id` があれば管理画面で足したキー（断られた回数を数える）、なければ秘密の設定のキー */
export interface RelayKey {
  id?: string;
  key: () => Promise<string>;
}

/** ベータを閉じている（管理画面で閉じた）。コードを出さず、中継もしない */
export async function betaClosed(db: Db): Promise<boolean> {
  const row = await db.select().from(betaSettings).where(eq(betaSettings.name, "closed")).get();
  return row?.value === "1";
}

/** 1 日の上限（ドル）。管理画面で変えていなければ、既定の値（DESIGN.md 17.3） */
export async function betaLimits(db: Db): Promise<{ dailyUsd: number; codeDailyUsd: number }> {
  const rows = await db
    .select()
    .from(betaSettings)
    .where(inArray(betaSettings.name, ["dailyUsd", "codeDailyUsd"]));
  const saved = (name: string, standard: number) => {
    const value = Number(rows.find((row) => row.name === name)?.value);
    return value > 0 ? value : standard;
  };
  return { dailyUsd: saved("dailyUsd", BETA_DAILY_USD), codeDailyUsd: saved("codeDailyUsd", CODE_DAILY_USD) };
}

/**
 * その会社に送るキーを、使う順に並べる（DESIGN.md 17.3）。管理画面で足した有効なキーを、最近断られていないものから。
 * 同じ条件のキーは毎回並べ替えて、負荷を分ける。最後に、秘密の設定のキー（あれば）
 */
export async function betaRelayKeys(db: Db, env: BetaEnv, provider: BetaProvider, secret: string): Promise<RelayKey[]> {
  const rows = await db
    .select({ id: betaKeys.id, sealed: betaKeys.encryptedKey, failedAt: betaKeys.failedAt })
    .from(betaKeys)
    .where(and(eq(betaKeys.provider, provider), eq(betaKeys.enabled, true)));
  const shuffled = rows
    .map((row) => ({ row, order: Math.random() }))
    .sort((a, b) => (a.row.failedAt ?? 0) - (b.row.failedAt ?? 0) || a.order - b.order);
  const keys: RelayKey[] = shuffled.map(({ row }) => ({ id: row.id, key: () => decrypt(row.sealed, secret) }));
  const fallback = provider === "qwen" ? env.BETA_QWEN_KEY : env.BETA_OPENROUTER_KEY;
  if (fallback) keys.push({ key: async () => fallback });
  return keys;
}

/** 開発者のキーがある会社。閉じていれば、どの会社もない */
export async function betaProviders(db: Db, env: BetaEnv): Promise<BetaProvider[]> {
  if (await betaClosed(db)) return [];
  const rows = await db
    .selectDistinct({ provider: betaKeys.provider })
    .from(betaKeys)
    .where(eq(betaKeys.enabled, true));
  const has = (p: BetaProvider) =>
    rows.some((r) => r.provider === p) || !!(p === "qwen" ? env.BETA_QWEN_KEY : env.BETA_OPENROUTER_KEY);
  return (["qwen", "openrouter"] as const).filter(has);
}

/** キーが外に断られたことを覚える（管理画面に出す。次は、ほかのキーを先に使う） */
export async function keyFailed(db: Db, id: string, status: number, now: number) {
  await db
    .update(betaKeys)
    .set({ failures: sql`${betaKeys.failures} + 1`, failedAt: now, failedStatus: status })
    .where(eq(betaKeys.id, id));
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
  const { codeDailyUsd } = await betaLimits(db);
  await db.insert(codes).values({
    id: newId(),
    codeHash: await sha256(normalizeCode(code)),
    dailyLimitUsd: codeDailyUsd,
    createdAt: now,
  });
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
    .where(and(eq(codes.id, codeId), sql`${spentToday} < COALESCE(${codes.dailyLimitUsd}, ${CODE_DAILY_USD})`))
    .returning({ id: codes.id })
    .get();
  if (!mine) return false;
  const { dailyUsd } = await betaLimits(db);
  const all = await db
    .insert(betaSpend)
    .values({ day: today, usd: estimate })
    .onConflictDoUpdate({
      target: betaSpend.day,
      set: { usd: sql`${betaSpend.usd} + ${estimate}` },
      setWhere: sql`${betaSpend.usd} < ${dailyUsd}`,
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
