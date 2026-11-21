// 管理画面（別の Worker。DESIGN.md 17.3）。ベータの費用とコードを見て、開発者のキーを管理する。
// 入口は Cloudflare Access（access.ts）。ここに届く要求は、Access を通った人のものだけを受ける

import { and, desc, eq, sql } from "drizzle-orm";
import { Hono } from "hono";
import { accessEmail, type AccessEnv } from "./access";
import { page, script } from "./adminPage";
import {
  BETA_CODES_PER_DAY,
  BETA_CODES_PER_IP,
  BETA_LIMIT_MAX_USD,
  betaClosed,
  betaLimits,
} from "./beta";
import { day, month, open, type Db } from "./db";
import { betaGrants, betaKeys, betaSettings, betaSpend, codes, codeUsage, tasks } from "./schema";
import { newId } from "./secrets";
import { allTasks, isModel, isTask, TASKS } from "./tasks";

/** API の Worker の `Vault`（worker.ts）。キーの暗号化を頼む */
export interface Vault {
  seal(plain: string): Promise<string>;
  fallbacks(): Promise<{ qwen: boolean; openrouter: boolean }>;
}

export interface AdminEnv extends AccessEnv {
  /** 手元で画面を確かめるときだけ使う（下の `local`） */
  ADMIN_DEV_EMAIL?: string;
  DB: D1Database;
  VAULT: Vault;
}

type App = { Bindings: AdminEnv; Variables: { db: Db; email: string } };

export const admin = new Hono<App>();

const PROVIDERS = ["qwen", "openrouter"] as const;
/** 一覧に出すコードの数（今日よく使った順） */
const LISTED_CODES = 100;
/** キーの長さ。短すぎるもの（打ち間違い）は、終わりの 4 文字を出すと中身が分かってしまう */
const MIN_KEY = 20;
const MAX_KEY = 400;
const MAX_MEMO = 100;

/** ページに書いたスクリプトだけを動かす（ほかのスクリプトが紛れ込んでも動かない）。枠の中に入れさせず、残させない */
admin.use(async (c, next) => {
  await next();
  const allowed = `'sha256-${await scriptHash()}'`;
  c.header(
    "Content-Security-Policy",
    `default-src 'none'; script-src ${allowed}; style-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'`,
  );
  c.header("X-Frame-Options", "DENY");
  c.header("Cache-Control", "no-store");
  c.header("Referrer-Policy", "no-referrer");
  c.header("X-Content-Type-Options", "nosniff");
});

let hashed: Promise<string> | undefined;
function scriptHash(): Promise<string> {
  hashed ??= crypto.subtle
    .digest("SHA-256", new TextEncoder().encode(script))
    .then((digest) => btoa(String.fromCharCode(...new Uint8Array(digest))));
  return hashed;
}

admin.get("/", (c) => c.html(page));

admin.use("/api/*", async (c, next) => {
  // 手元で画面を確かめるとき（`wrangler dev -c wrangler.admin.jsonc --var ADMIN_DEV_EMAIL:…`）だけ、Access を通さない。
  // 本番のホストは localhost にならないので、設定を入れても効かない
  const host = new URL(c.req.url).hostname;
  const local = host === "localhost" || host === "127.0.0.1" ? c.env.ADMIN_DEV_EMAIL : undefined;
  if (!local && (!c.env.TEAM_DOMAIN || !c.env.POLICY_AUD)) return c.json({ error: "notConfigured" }, 503);
  const email = local ?? (await accessEmail(c.req.raw, c.env));
  if (!email) return c.json({ error: "unauthorized" }, 401);
  // Access のクッキーは、ほかのサイトからの要求にも付く。書き換える要求は、このページの JavaScript が付ける
  // ヘッダーがあるものだけを受ける（ほかのサイトからは、このヘッダーを付けて送れない）
  if (c.req.method !== "GET" && c.req.header("X-Admin") !== "1") return c.json({ error: "forbidden" }, 403);
  c.set("db", open(c.env.DB));
  c.set("email", email);
  await next();
});

/** アプリの版ごとの数を、何日前までのコードで数えるか（毎日は配信しない人も入るように、2 週間） */
const VERSION_DAYS = 14;
const DAY_MS = 24 * 60 * 60 * 1000;

/** 今の状態。費用（今日と今月）、コードの数、上限、閉じているか、使われているアプリの版 */
admin.get("/api/summary", async (c) => {
  const db = c.var.db;
  const now = Date.now();
  const today = day(now);
  const [spend, monthUsd, total, active, granted, closed, limits, versions] = await Promise.all([
    db.select({ usd: betaSpend.usd }).from(betaSpend).where(eq(betaSpend.day, today)).get(),
    db
      .select({ usd: sql<number>`COALESCE(SUM(${codeUsage.usd}), 0)` })
      .from(codeUsage)
      .where(eq(codeUsage.month, month(now)))
      .get(),
    db.select({ n: sql<number>`COUNT(*)` }).from(codes).get(),
    db
      .select({ n: sql<number>`COUNT(*)` })
      .from(codes)
      .where(and(eq(codes.day, today), sql`${codes.dayUsd} > 0`))
      .get(),
    db
      .select({ n: betaGrants.count })
      .from(betaGrants)
      .where(and(eq(betaGrants.day, today), eq(betaGrants.place, "*")))
      .get(),
    betaClosed(db),
    betaLimits(db),
    // この 14 日にアプリが確かめに来たコード（止めたものは除く）の、アプリの版ごとの数（版を伝えない 0.9.6 までは null）
    db
      .select({ version: codes.appVersion, codes: sql<number>`COUNT(*)` })
      .from(codes)
      .where(and(eq(codes.revoked, false), sql`${codes.seenDay} >= ${day(now - VERSION_DAYS * DAY_MS)}`))
      .groupBy(codes.appVersion)
      .orderBy(desc(sql`COUNT(*)`), desc(codes.appVersion)),
  ]);
  return c.json({
    email: c.var.email,
    closed,
    today: { usd: spend?.usd ?? 0, limitUsd: limits.dailyUsd, codes: granted?.n ?? 0, codesLimit: BETA_CODES_PER_DAY },
    month: { usd: monthUsd?.usd ?? 0 },
    codes: { total: total?.n ?? 0, activeToday: active?.n ?? 0 },
    limits: { codeDailyUsd: limits.codeDailyUsd, codesPerPlace: BETA_CODES_PER_IP, maxUsd: BETA_LIMIT_MAX_USD },
    versions: { days: VERSION_DAYS, rows: versions },
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
    .where(eq(codes.id, c.req.param("id")));
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
      failedReason: betaKeys.failedReason,
      createdAt: betaKeys.createdAt,
    })
    .from(betaKeys)
    .orderBy(betaKeys.provider, betaKeys.createdAt);
  return c.json({ keys, fallback: await c.env.VAULT.fallbacks() });
});

admin.post("/api/keys", async (c) => {
  const body = await c.req.json<Record<string, unknown>>().catch(() => ({}) as Record<string, unknown>);
  const provider = PROVIDERS.find((p) => p === body.provider);
  const key = typeof body.key === "string" ? body.key.trim() : "";
  const memo = typeof body.memo === "string" ? body.memo.trim() : "";
  if (!provider || key.length < MIN_KEY || key.length > MAX_KEY || memo.length > MAX_MEMO) {
    return c.json({ error: "invalid" }, 400);
  }
  const id = newId();
  await c.var.db.insert(betaKeys).values({
    id,
    provider,
    encryptedKey: await c.env.VAULT.seal(key),
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
  const cleared = enabled ? { failures: 0, failedAt: null, failedStatus: null, failedReason: null } : {};
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

/** 仕事の口（/v2。DESIGN.md 17.4）ごとの、今のモデルと、選べるモデル、動かしているか */
admin.get("/api/tasks", async (c) => {
  const all = await allTasks(c.var.db);
  const listed = all.map(({ task, id, enabled }) => {
    const { label, models } = TASKS[task];
    const options = Object.entries(models).map(([model, { label }]) => ({ id: model, label }));
    return { name: task, label, model: id, enabled, models: options };
  });
  return c.json({ tasks: listed });
});

/** 仕事のモデルを替える、止める・動かす。送った項目だけを変える。次の要求から効く（アプリのリリースは要らない） */
admin.patch("/api/tasks/:name", async (c) => {
  const name = c.req.param("name");
  if (!isTask(name)) return c.json({ error: "notFound" }, 404);
  const body = await c.req.json<Record<string, unknown>>().catch(() => ({}) as Record<string, unknown>);
  const model = typeof body.model === "string" && isModel(name, body.model) ? body.model : undefined;
  const enabled = typeof body.enabled === "boolean" ? body.enabled : undefined;
  const sent = Object.keys(body).length;
  if (sent === 0 || sent !== [model, enabled].filter((value) => value !== undefined).length) {
    return c.json({ error: "invalid" }, 400);
  }
  const now = (await allTasks(c.var.db)).find((t) => t.task === name)!;
  const row = { name, model: model ?? now.id, enabled: enabled ?? now.enabled };
  await c.var.db.insert(tasks).values(row).onConflictDoUpdate({ target: tasks.name, set: row });
  return c.body(null, 204);
});

/**
 * ベータの設定を変える。送った項目だけを変える。`closed` は、閉じるとコードを出さず、中継もしない（出したコードは残る）。
 * `dailyUsd` はベータ全体の 1 日の上限、`codeDailyUsd` は 1 つのコードの 1 日の上限（出してあるコードにも当てる）
 */
admin.put("/api/settings", async (c) => {
  type Body = { closed?: unknown; dailyUsd?: unknown; codeDailyUsd?: unknown };
  const body = await c.req.json<Body>().catch(() => ({}) as Body);
  const usd = (value: unknown) =>
    typeof value === "number" && value > 0 && value <= BETA_LIMIT_MAX_USD ? String(value) : undefined;
  const values = {
    closed: typeof body.closed === "boolean" ? (body.closed ? "1" : "0") : undefined,
    dailyUsd: usd(body.dailyUsd),
    codeDailyUsd: usd(body.codeDailyUsd),
  };
  const sent = Object.entries(body).filter(([, value]) => value !== undefined);
  const known = Object.entries(values).filter(([, value]) => value !== undefined) as [string, string][];
  if (known.length === 0 || known.length !== sent.length) return c.json({ error: "invalid" }, 400);
  const db = c.var.db;
  for (const [name, value] of known) {
    await db
      .insert(betaSettings)
      .values({ name, value })
      .onConflictDoUpdate({ target: betaSettings.name, set: { value } });
  }
  if (values.codeDailyUsd) {
    await db.update(codes).set({ dailyLimitUsd: Number(values.codeDailyUsd) });
  }
  return c.body(null, 204);
});
