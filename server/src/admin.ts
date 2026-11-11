// 管理画面（admin.jimaku.deadlock.jp。DESIGN.md 17.3）。ベータの費用とコードを見て、開発者のキーを管理する。
// 入口は Cloudflare Access（access.ts）。ここに届く要求は、Access を通った人のものだけを受ける

import { and, desc, eq, sql } from "drizzle-orm";
import { Hono } from "hono";
import { accessEmail, type AccessEnv } from "./access";
import { page } from "./adminPage";
import {
  BETA_CODE_DAILY_USD,
  BETA_CODES_PER_DAY,
  BETA_CODES_PER_IP,
  BETA_DAILY_USD,
  BETA_ISSUER,
  betaClosed,
  type BetaEnv,
} from "./beta";
import { day, month, open, type Db } from "./db";
import { betaGrants, betaKeys, betaSettings, betaSpend, codes, codeUsage } from "./schema";
import { encrypt, newId } from "./secrets";

export interface AdminEnv extends AccessEnv, BetaEnv {
  /** 手元で画面を確かめるときだけ使う（上の `local`） */
  ADMIN_DEV_EMAIL?: string;
  DB: D1Database;
  SHARE_KEY: string;
}

type App = { Bindings: AdminEnv; Variables: { db: Db; email: string } };

export const admin = new Hono<App>();

const PROVIDERS = ["qwen", "openrouter"] as const;
/** 一覧に出すコードの数（今日よく使った順） */
const LISTED_CODES = 100;
const MAX_KEY = 400;
const MAX_MEMO = 100;

admin.get("/", (c) => c.html(page));

admin.use("/api/*", async (c, next) => {
  // 手元で画面を確かめるとき（`wrangler dev --var ADMIN_DEV_EMAIL:…` で admin.localhost を開く）だけ、Access を通さない。
  // 本番のホストは admin.localhost にならないので、設定を入れても効かない
  const local = new URL(c.req.url).hostname === "admin.localhost" ? c.env.ADMIN_DEV_EMAIL : undefined;
  if (!local && (!c.env.ACCESS_TEAM || !c.env.ACCESS_AUD)) return c.json({ error: "notConfigured" }, 503);
  const email = local ?? (await accessEmail(c.req.raw, c.env));
  if (!email) return c.json({ error: "unauthorized" }, 401);
  // Access のクッキーは、ほかのサイトからの要求にも付く。書き換える要求は、このページの JavaScript が付ける
  // ヘッダーがあるものだけを受ける（ほかのサイトからは、このヘッダーを付けて送れない）
  if (c.req.method !== "GET" && c.req.header("X-Admin") !== "1") return c.json({ error: "forbidden" }, 403);
  c.set("db", open(c.env.DB));
  c.set("email", email);
  await next();
});

/** 今の状態。費用（今日と今月）、コードの数、上限、閉じているか */
admin.get("/api/summary", async (c) => {
  const db = c.var.db;
  const now = Date.now();
  const today = day(now);
  const [spend, monthUsd, total, active, granted, closed] = await Promise.all([
    db.select({ usd: betaSpend.usd }).from(betaSpend).where(eq(betaSpend.day, today)).get(),
    db
      .select({ usd: sql<number>`COALESCE(SUM(${codeUsage.usd}), 0)` })
      .from(codeUsage)
      .innerJoin(codes, eq(codes.id, codeUsage.codeId))
      .where(and(eq(codes.issuerId, BETA_ISSUER), eq(codeUsage.month, month(now))))
      .get(),
    db.select({ n: sql<number>`COUNT(*)` }).from(codes).where(eq(codes.issuerId, BETA_ISSUER)).get(),
    db
      .select({ n: sql<number>`COUNT(*)` })
      .from(codes)
      .where(and(eq(codes.issuerId, BETA_ISSUER), eq(codes.day, today), sql`${codes.dayUsd} > 0`))
      .get(),
    db
      .select({ n: betaGrants.count })
      .from(betaGrants)
      .where(and(eq(betaGrants.day, today), eq(betaGrants.place, "*")))
      .get(),
    betaClosed(db),
  ]);
  return c.json({
    email: c.var.email,
    closed,
    today: { usd: spend?.usd ?? 0, limitUsd: BETA_DAILY_USD, codes: granted?.n ?? 0, codesLimit: BETA_CODES_PER_DAY },
    month: { usd: monthUsd?.usd ?? 0 },
    codes: { total: total?.n ?? 0, activeToday: active?.n ?? 0 },
    limits: { codeDailyUsd: BETA_CODE_DAILY_USD, codesPerPlace: BETA_CODES_PER_IP },
  });
});

/** ベータのコード。今日よく使った順 */
admin.get("/api/codes", async (c) => {
  const now = Date.now();
  const today = day(now);
  const todayUsd = sql<number>`CASE WHEN ${codes.day} = ${today} THEN ${codes.dayUsd} ELSE 0 END`;
  const rows = await c.var.db
    .select({
      id: codes.id,
      createdAt: codes.createdAt,
      revoked: codes.revoked,
      todayUsd,
      monthUsd: sql<number>`COALESCE(SUM(CASE WHEN ${codeUsage.month} = ${month(now)} THEN ${codeUsage.usd} END), 0)`,
      usd: sql<number>`COALESCE(SUM(${codeUsage.usd}), 0)`,
    })
    .from(codes)
    .leftJoin(codeUsage, eq(codeUsage.codeId, codes.id))
    .where(eq(codes.issuerId, BETA_ISSUER))
    .groupBy(codes.id)
    .orderBy(desc(todayUsd), desc(codes.createdAt))
    .limit(LISTED_CODES);
  return c.json({ codes: rows });
});

/** コードを止める・戻す */
admin.patch("/api/codes/:id", async (c) => {
  const body = await c.req.json<{ revoked?: unknown }>().catch(() => ({}) as { revoked?: unknown });
  if (typeof body.revoked !== "boolean") return c.json({ error: "invalid" }, 400);
  const { meta } = await c.var.db
    .update(codes)
    .set({ revoked: body.revoked })
    .where(and(eq(codes.id, c.req.param("id")), eq(codes.issuerId, BETA_ISSUER)));
  return meta.changes ? c.body(null, 204) : c.json({ error: "notFound" }, 404);
});

/** 開発者のキー。キーそのものは返さない（終わりの 4 文字だけ） */
admin.get("/api/keys", async (c) => {
  const keys = await c.var.db
    .select({
      id: betaKeys.id,
      provider: betaKeys.provider,
      tail: betaKeys.tail,
      memo: betaKeys.memo,
      enabled: betaKeys.enabled,
      failures: betaKeys.failures,
      failedAt: betaKeys.failedAt,
      failedStatus: betaKeys.failedStatus,
      createdAt: betaKeys.createdAt,
    })
    .from(betaKeys)
    .orderBy(betaKeys.provider, betaKeys.createdAt);
  // 秘密の設定（wrangler）に入れたキーがあるか。管理画面のキーが断られたときの最後の頼み
  const fallback = { qwen: !!c.env.BETA_QWEN_KEY, openrouter: !!c.env.BETA_OPENROUTER_KEY };
  return c.json({ keys, fallback });
});

admin.post("/api/keys", async (c) => {
  const body = await c.req.json<Record<string, unknown>>().catch(() => ({}) as Record<string, unknown>);
  const provider = PROVIDERS.find((p) => p === body.provider);
  const key = typeof body.key === "string" ? body.key.trim() : "";
  const memo = typeof body.memo === "string" ? body.memo.trim() : "";
  if (!provider || !key || key.length > MAX_KEY || memo.length > MAX_MEMO) return c.json({ error: "invalid" }, 400);
  const id = newId();
  await c.var.db.insert(betaKeys).values({
    id,
    provider,
    encryptedKey: await encrypt(key, c.env.SHARE_KEY),
    tail: key.slice(-4),
    memo,
    createdAt: Date.now(),
  });
  return c.json({ id });
});

/** キーを止める・戻す、メモを変える。戻すときは、断られた記録を消す（また先に使う） */
admin.patch("/api/keys/:id", async (c) => {
  const body = await c.req.json<Record<string, unknown>>().catch(() => ({}) as Record<string, unknown>);
  const enabled = typeof body.enabled === "boolean" ? body.enabled : undefined;
  const memo = typeof body.memo === "string" && body.memo.length <= MAX_MEMO ? body.memo.trim() : undefined;
  if (enabled === undefined && memo === undefined) return c.json({ error: "invalid" }, 400);
  const cleared = enabled ? { failures: 0, failedAt: null, failedStatus: null } : {};
  const { meta } = await c.var.db
    .update(betaKeys)
    .set({ enabled, memo, ...cleared })
    .where(eq(betaKeys.id, c.req.param("id")));
  return meta.changes ? c.body(null, 204) : c.json({ error: "notFound" }, 404);
});

admin.delete("/api/keys/:id", async (c) => {
  await c.var.db.delete(betaKeys).where(eq(betaKeys.id, c.req.param("id")));
  return c.body(null, 204);
});

/** ベータを閉じる・開ける。閉じると、コードを出さず、中継もしない（出したコードは残る） */
admin.put("/api/settings", async (c) => {
  const body = await c.req.json<{ closed?: unknown }>().catch(() => ({}) as { closed?: unknown });
  if (typeof body.closed !== "boolean") return c.json({ error: "invalid" }, 400);
  const value = body.closed ? "1" : "0";
  await c.var.db
    .insert(betaSettings)
    .values({ name: "closed", value })
    .onConflictDoUpdate({ target: betaSettings.name, set: { value } });
  return c.body(null, 204);
});
