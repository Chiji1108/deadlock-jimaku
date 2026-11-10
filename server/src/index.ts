// API（api.jimaku.deadlock.jp。DESIGN.md 17.1）。パスは版（/v1）で区切る。
// おすそ分け: 自分の OpenAI キーを預けた人がコードを作り、コードを持つ人の通信を、そのキーで OpenAI に中継する。
// xAI のキーも預けていれば、確定の文字起こし（Grok）を xAI に中継する
import { and, desc, eq, sql } from "drizzle-orm";
import { Hono, type Context } from "hono";
import {
  addUsage,
  codeByValue,
  dailyLimit,
  issuerByKey,
  issuerByToken,
  issuerProviders,
  month,
  overLimit,
  providers,
  refusal,
  todayUsd,
  open,
  PROVIDERS,
  type Code,
  type Db,
  type Issuer,
} from "./db";
import { openai, validKey } from "./openai";
import { audioUsd, tokensOf, tokensUsd, wavSeconds, type Provider } from "./price";
import { relay } from "./realtime";
import { codes, codeUsage, issuerKeys, issuers } from "./schema";
import { decrypt, encrypt, issuerToken, newCode, newId, normalizeCode, sha256 } from "./secrets";
import { validXaiKey, xai } from "./xai";

export interface Env {
  DB: D1Database;
  /** キーを暗号化する鍵（32 バイトの base64）。`wrangler secret put SHARE_KEY` */
  SHARE_KEY: string;
  /** コードごとの 1 分あたりの要求数の上限（値は wrangler.jsonc。DESIGN.md 17.1） */
  GIFT_RATE: RateLimit;
}

type App = { Bindings: Env; Variables: { db: Db } };
type Ctx = Context<App>;

export const app = new Hono<App>().basePath("/v1");

app.use(async (c, next) => {
  c.set("db", open(c.env.DB));
  await next();
});

/** memo の長さの上限 */
const MAX_MEMO = 100;

/** Responses API に渡す項目。ほかの項目（有料のツールなど）は、おすそ分けした人の料金を数えられないので通さない */
const RESPONSES_FIELDS = ["model", "instructions", "input", "text", "reasoning", "max_output_tokens", "stream"];

/**
 * 1 回の出力の上限（推論の分も含む）。アプリは送らないので、ここで必ず付ける。切れると JSON が読めなくなるので、
 * いちばん長い辞書の読み（40 語で 3,000 トークンほど）の 10 倍にする（DESIGN.md 17.1）
 */
const MAX_OUTPUT_TOKENS = 32_000;

/** xAI の文字起こしに渡す項目（アプリが送るもの）。xAI はファイルより後ろの欄を読まないことがあるので、ファイルは最後に足す */
const XAI_STT_FIELDS = ["model", "language", "keyterm"];

/** 本文の長さの上限（文字数）。いちばん長い用語の判定（辞書の全部を送る。32,000 文字ほど）の 8 倍 */
const MAX_RESPONSES_BODY = 256_000;

/** Decisions API が使えるモデル（これだけ） */
const DECISIONS_MODEL = "gpt-6-luna";

/** Decisions の本文と問いの数の上限。アプリは 1 発話で候補の数だけ問い（多くて 10 ほど、1 問 1,000 文字ほど）を送る */
const MAX_DECISIONS_BODY = 64_000;
const MAX_DECISIONS_QUESTIONS = 40;

/** ストリームの最後の応答の usage */
function streamUsage(text: string): unknown {
  for (const line of text.split("\n").reverse()) {
    const usage = line.startsWith("data:") ? parsedUsage(line.slice("data:".length)) : undefined;
    if (usage) return usage;
  }
  return undefined;
}

function parsedUsage(data: string): unknown {
  try {
    return (JSON.parse(data) as { response?: { usage?: unknown } }).response?.usage;
  } catch {
    return undefined;
  }
}

// ---- おすそ分けする人（合言葉で見分ける） ----

/** 会社ごとのキーの確かめ方と、使えないときのエラー */
const KEY_CHECKS: Record<Provider, { valid: (key: string) => Promise<boolean>; error: string }> = {
  openai: { valid: validKey, error: "invalidKey" },
  xai: { valid: validXaiKey, error: "invalidXaiKey" },
};

function provider(value: unknown): Provider | undefined {
  return PROVIDERS.find((p) => p === value);
}

/** 合言葉を決め直して返す（id と tokenSalt で決まるので、どの PC でも同じものになる） */
async function issueToken(c: Ctx, issuer: Issuer): Promise<string> {
  const token = await issuerToken(issuer.id, issuer.tokenSalt, c.env.SHARE_KEY);
  await c.var.db
    .update(issuers)
    .set({ adminTokenHash: await sha256(token) })
    .where(eq(issuers.id, issuer.id));
  return token;
}

/** 本文のキーを確かめる。使えなければ返す応答 */
async function checkedKey(c: Ctx, p: Provider, key: unknown): Promise<string | Response> {
  if (typeof key !== "string" || !key) return c.json({ error: "noKey" }, 400);
  if (!(await KEY_CHECKS[p].valid(key))) return c.json({ error: KEY_CHECKS[p].error }, 400);
  return key;
}

/**
 * キーを預けておすそ分けを始める。預けたキーのどれかと同じキーなら、どの PC からでも同じ人になり、同じ合言葉とコードを使う。
 * provider を省いた本文（0.4 までのアプリ）は OpenAI のキー。join なら新しく始めず、預けていないキーは 404
 */
app.post("/share", async (c) => {
  const body = (await json<{ provider?: unknown; key?: unknown; join?: unknown }>(c)) ?? {};
  const p = body.provider === undefined ? "openai" : provider(body.provider);
  if (!p) return c.json({ error: "invalid" }, 400);
  const key = await checkedKey(c, p, body.key);
  if (key instanceof Response) return key;
  const keyHash = await sha256(key);
  const found = await issuerByKey(c.var.db, keyHash);
  if (found) return c.json({ token: await issueToken(c, found) });
  if (body.join === true) return c.json({ error: "notShared" }, 404);
  const issuer = { id: newId(), tokenSalt: newId() };
  const token = await issuerToken(issuer.id, issuer.tokenSalt, c.env.SHARE_KEY);
  await c.var.db.batch([
    c.var.db.insert(issuers).values({ ...issuer, adminTokenHash: await sha256(token), createdAt: Date.now() }),
    c.var.db.insert(issuerKeys).values({
      issuerId: issuer.id,
      provider: p,
      keyHash,
      encryptedKey: await encrypt(key, c.env.SHARE_KEY),
    }),
  ]);
  return c.json({ token });
});

/**
 * その会社のキーを預ける・替える。替えたときは合言葉も新しくなり、古いキーで得た合言葉は使えなくなる。
 * 新しい合言葉を返す（0.4 までのアプリは PUT /share/key で OpenAI のキーを替える）
 */
async function putKey(c: Ctx, p: Provider) {
  const issuer = await admin(c);
  if (!issuer) return c.json({ error: "unauthorized" }, 401);
  const key = await checkedKey(c, p, (await json<{ key?: unknown }>(c))?.key);
  if (key instanceof Response) return key;
  const keyHash = await sha256(key);
  const owner = await issuerByKey(c.var.db, keyHash);
  // そのキーで別の人としておすそ分けを有効にしている。片方を無効にしてもらう
  if (owner && owner.id !== issuer.id) return c.json({ error: "keyInUse" }, 409);
  if (owner) return c.json({ token: await issueToken(c, issuer) });
  const replaced = (await issuerProviders(c.var.db, issuer.id)).includes(p);
  const tokenSalt = replaced ? newId() : issuer.tokenSalt;
  const encryptedKey = await encrypt(key, c.env.SHARE_KEY);
  await c.var.db.batch([
    c.var.db
      .insert(issuerKeys)
      .values({ issuerId: issuer.id, provider: p, keyHash, encryptedKey })
      .onConflictDoUpdate({ target: [issuerKeys.issuerId, issuerKeys.provider], set: { keyHash, encryptedKey } }),
    c.var.db.update(issuers).set({ tokenSalt }).where(eq(issuers.id, issuer.id)),
  ]);
  return c.json({ token: await issueToken(c, { ...issuer, tokenSalt }) });
}

app.put("/share/keys/:provider", async (c) => {
  const p = provider(c.req.param("provider"));
  return p ? putKey(c, p) : c.json({ error: "notFound" }, 404);
});

app.put("/share/key", (c) => putKey(c, "openai"));

/** 預けたキーを消す。最後の 1 つは消せない（おすそ分けをやめる）。消したキーで得た合言葉を残さないよう、新しい合言葉を返す */
app.delete("/share/keys/:provider", async (c) => {
  const issuer = await admin(c);
  if (!issuer) return c.json({ error: "unauthorized" }, 401);
  const p = provider(c.req.param("provider"));
  if (!p) return c.json({ error: "notFound" }, 404);
  const deposited = await issuerProviders(c.var.db, issuer.id);
  if (!deposited.includes(p)) return c.json({ token: await issueToken(c, issuer) });
  if (deposited.length === 1) return c.json({ error: "lastKey" }, 409);
  const tokenSalt = newId();
  await c.var.db.batch([
    c.var.db.delete(issuerKeys).where(and(eq(issuerKeys.issuerId, issuer.id), eq(issuerKeys.provider, p))),
    c.var.db.update(issuers).set({ tokenSalt }).where(eq(issuers.id, issuer.id)),
  ]);
  return c.json({ token: await issueToken(c, { ...issuer, tokenSalt }) });
});

/** おすそ分けをやめる。預けたキーとコードを消す */
app.delete("/share", async (c) => {
  const issuer = await admin(c);
  if (!issuer) return c.json({ error: "unauthorized" }, 401);
  await c.var.db.delete(issuers).where(eq(issuers.id, issuer.id));
  return c.body(null, 204);
});

app.get("/share/codes", async (c) => {
  const issuer = await admin(c);
  if (!issuer) return c.json({ error: "unauthorized" }, 401);
  const now = Date.now();
  const rows = await c.var.db
    .select({
      id: codes.id,
      memo: codes.memo,
      expiresAt: codes.expiresAt,
      revoked: codes.revoked,
      createdAt: codes.createdAt,
      dailyLimitUsd: codes.dailyLimitUsd,
      day: codes.day,
      dayUsd: codes.dayUsd,
      usd: sql<number>`COALESCE(SUM(${codeUsage.usd}), 0)`,
      monthUsd: sql<number>`COALESCE(SUM(CASE WHEN ${codeUsage.month} = ${month(now)} THEN ${codeUsage.usd} END), 0)`,
    })
    .from(codes)
    .leftJoin(codeUsage, eq(codeUsage.codeId, codes.id))
    .where(eq(codes.issuerId, issuer.id))
    .groupBy(codes.id)
    .orderBy(desc(codes.createdAt));
  // dailyLimitUsd は決めた額（null は既定）、effectiveLimitUsd は実際に効く額
  const list = rows.map(({ day, ...r }) => ({
    ...r,
    dayUsd: todayUsd({ day, dayUsd: r.dayUsd }, now),
    effectiveLimitUsd: dailyLimit(r),
  }));
  // providers: キーを預けている会社（コードはどれもこの会社に使える）
  return c.json({ codes: list, providers: await issuerProviders(c.var.db, issuer.id) });
});

/** コードを作る。コードそのものはこの応答でだけ返す（保存するのはハッシュ） */
app.post("/share/codes", async (c) => {
  const issuer = await admin(c);
  if (!issuer) return c.json({ error: "unauthorized" }, 401);
  const fields = codeFields((await json(c)) ?? {});
  if (!fields) return c.json({ error: "invalid" }, 400);
  const id = newId();
  const code = newCode();
  await c.var.db.insert(codes).values({
    id,
    codeHash: await sha256(normalizeCode(code)),
    issuerId: issuer.id,
    memo: fields.memo,
    expiresAt: fields.expiresAt,
    dailyLimitUsd: fields.dailyLimitUsd,
    createdAt: Date.now(),
  });
  return c.json({ id, code });
});

/** メモ・期限・1 日の上限を変える。止める（revoked）・止めたのを戻す */
app.patch("/share/codes/:id", async (c) => {
  const issuer = await admin(c);
  if (!issuer) return c.json({ error: "unauthorized" }, 401);
  const body = (await json<CodeFields & { revoked?: unknown }>(c)) ?? {};
  const fields = codeFields(body);
  if (!fields || (body.revoked !== undefined && typeof body.revoked !== "boolean")) {
    return c.json({ error: "invalid" }, 400);
  }
  // undefined の欄は変えない
  const set = { ...fields, revoked: body.revoked as boolean | undefined };
  if (Object.values(set).every((v) => v === undefined)) return c.body(null, 204);
  const { meta } = await c.var.db
    .update(codes)
    .set(set)
    .where(and(eq(codes.id, c.req.param("id")), eq(codes.issuerId, issuer.id)));
  return meta.changes ? c.body(null, 204) : c.json({ error: "notFound" }, 404);
});

// ---- コードを使う人 ----

/** コードの状態。アプリは動いていない字幕を始めるときに、これで期限を確かめる（ここでは猶予を足さない） */
app.get("/gift", async (c) => {
  const code = await giftCode(c);
  if (!code) return c.json({ error: "unknown" }, 401);
  if (code.revoked) return c.json({ error: "revoked" }, 403);
  if (code.expiresAt !== null && Date.now() >= code.expiresAt) {
    return c.json({ error: "expired", expiresAt: code.expiresAt }, 403);
  }
  if (overLimit(code, Date.now())) return c.json({ error: "limit" }, 403);
  return c.json({ expiresAt: code.expiresAt, providers: providers(code) });
});

app.post("/openai/responses", async (c) => {
  const gift = await giftKey(c);
  if (gift instanceof Response) return gift;
  const text = await c.req.text();
  if (text.length > MAX_RESPONSES_BODY) return c.json({ error: "tooLarge" }, 413);
  const body = parsed<Record<string, unknown>>(text) ?? {};
  const model = body.model;
  // input はアプリが送る文だけ（配列なら画像やファイルを渡せて、長さの上限が効かない）
  if (
    typeof model !== "string" ||
    tokensUsd(model, tokensOf(undefined)) === undefined ||
    typeof body.input !== "string"
  ) {
    return c.json({ error: "notAllowed" }, 400);
  }
  const forwarded = Object.fromEntries(RESPONSES_FIELDS.flatMap((k) => (k in body ? [[k, body[k]]] : [])));
  const asked = body.max_output_tokens;
  const maxOutput = typeof asked === "number" ? Math.min(asked, MAX_OUTPUT_TOKENS) : MAX_OUTPUT_TOKENS;
  // 他人の字幕を通すので、OpenAI に応答を保存させない
  const res = await openai("/v1/responses", gift.key, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ...forwarded, max_output_tokens: maxOutput, store: false }),
  });
  // ストリームは流しながら写しを読み、最後の応答の usage で数える
  if (res.ok && body.stream === true && res.body) {
    const [client, copy] = res.body.tee();
    const usage = new Response(copy).text().then(streamUsage);
    c.executionCtx.waitUntil(
      usage.then((u) => addUsage(c.var.db, gift.code.id, tokensUsd(model, tokensOf(u)) ?? 0, Date.now())),
    );
    return new Response(client, { headers: { "Content-Type": "text/event-stream" } });
  }
  const answer = await res.text();
  if (res.ok) {
    const usage = (JSON.parse(answer) as { usage?: unknown }).usage;
    const usd = tokensUsd(model, tokensOf(usage)) ?? 0;
    c.executionCtx.waitUntil(addUsage(c.var.db, gift.code.id, usd, Date.now()));
  }
  return new Response(answer, { status: res.status, headers: { "Content-Type": "application/json" } });
});

/**
 * Decisions API（用語を確かめる。DESIGN.md 9.7）。アプリが送る形だけを通す: 文字の input と、文字だけの選択の問い。
 * 出力は課金されず、入力は Luna の単価で数える
 */
app.post("/openai/decisions", async (c) => {
  const gift = await giftKey(c);
  if (gift instanceof Response) return gift;
  const text = await c.req.text();
  if (text.length > MAX_DECISIONS_BODY) return c.json({ error: "tooLarge" }, 413);
  const body = parsed<Record<string, unknown>>(text) ?? {};
  const { model, input, questions } = body;
  // 画像やファイルを渡せる形（配列の input、部品の配列の instructions）は、長さの上限が効かないので通さない
  const choice = (q: unknown) => {
    const { type, name, instructions, choices } = (q ?? {}) as Record<string, unknown>;
    const option = (o: unknown) => {
      const { value, description } = (o ?? {}) as Record<string, unknown>;
      return typeof value === "string" && typeof description === "string";
    };
    return (
      type === "choice" &&
      typeof name === "string" &&
      typeof instructions === "string" &&
      Array.isArray(choices) &&
      choices.every(option)
    );
  };
  if (
    model !== DECISIONS_MODEL ||
    typeof input !== "string" ||
    !Array.isArray(questions) ||
    questions.length === 0 ||
    questions.length > MAX_DECISIONS_QUESTIONS ||
    !questions.every(choice)
  ) {
    return c.json({ error: "notAllowed" }, 400);
  }
  const res = await openai("/v1/decisions", gift.key, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ model, input, questions }),
  });
  const answer = await res.text();
  if (res.ok) {
    const usage = (JSON.parse(answer) as { usage?: unknown }).usage;
    const usd = tokensUsd(DECISIONS_MODEL, { ...tokensOf(usage), output: 0 }) ?? 0;
    c.executionCtx.waitUntil(addUsage(c.var.db, gift.code.id, usd, Date.now()));
  }
  return new Response(answer, { status: res.status, headers: { "Content-Type": "application/json" } });
});

app.post("/openai/audio/transcriptions", async (c) => {
  const gift = await giftKey(c);
  if (gift instanceof Response) return gift;
  const form = await c.req.formData().catch(() => undefined);
  if (!form) return c.json({ error: "invalid" }, 400);
  const model = form.get("model");
  const file = form.get("file");
  if (typeof model !== "string" || !(file instanceof File)) return c.json({ error: "invalid" }, 400);
  const seconds = wavSeconds(await file.arrayBuffer());
  const usd = seconds === undefined ? undefined : audioUsd("openai", model, seconds);
  if (usd === undefined) return c.json({ error: "notAllowed" }, 400);
  const res = await openai("/v1/audio/transcriptions", gift.key, { method: "POST", body: form });
  if (res.ok) c.executionCtx.waitUntil(addUsage(c.var.db, gift.code.id, usd, Date.now()));
  return new Response(res.body, {
    status: res.status,
    headers: { "Content-Type": res.headers.get("Content-Type") ?? "application/json" },
  });
});

app.post("/xai/stt", async (c) => {
  const gift = await giftKey(c, "xai");
  if (gift instanceof Response) return gift;
  const form = await c.req.formData().catch(() => undefined);
  if (!form) return c.json({ error: "invalid" }, 400);
  const model = form.get("model");
  const file = form.get("file");
  if (typeof model !== "string" || !(file instanceof File)) return c.json({ error: "invalid" }, 400);
  const seconds = wavSeconds(await file.arrayBuffer());
  const usd = seconds === undefined ? undefined : audioUsd("xai", model, seconds);
  if (usd === undefined) return c.json({ error: "notAllowed" }, 400);
  const forwarded = new FormData();
  for (const name of XAI_STT_FIELDS) {
    for (const value of form.getAll(name)) if (typeof value === "string") forwarded.append(name, value);
  }
  forwarded.append("file", file);
  const res = await xai("/v1/stt", gift.key, { method: "POST", body: forwarded });
  if (res.ok) c.executionCtx.waitUntil(addUsage(c.var.db, gift.code.id, usd, Date.now()));
  return new Response(res.body, {
    status: res.status,
    headers: { "Content-Type": res.headers.get("Content-Type") ?? "application/json" },
  });
});

app.get("/openai/realtime", async (c) => {
  if (c.req.header("Upgrade") !== "websocket") return c.json({ error: "websocket" }, 426);
  const gift = await giftKey(c);
  if (gift instanceof Response) return gift;
  return relay(c.var.db, gift.code, gift.key, c.executionCtx);
});

// ---- 共通 ----

function bearer(c: Ctx): string | undefined {
  return c.req.header("Authorization")?.match(/^Bearer (.+)$/)?.[1];
}

async function admin(c: Ctx) {
  const token = bearer(c);
  return token ? issuerByToken(c.var.db, token) : null;
}

async function giftCode(c: Ctx): Promise<Code | null> {
  const code = bearer(c);
  return code ? codeByValue(c.var.db, code) : null;
}

/**
 * 中継に使えるコードと、おすそ分けした人のその会社のキー。使えなければ返す応答。
 * 理由はヘッダーにも載せる（WebSocket のクライアントは、断られたときの本文を読みきれるとは限らない）。
 * 429 は 403 のあとに見るので、止まる理由を隠さない
 */
async function giftKey(
  c: Ctx,
  provider: Provider = "openai",
): Promise<{ code: Code; key: string } | Response> {
  const code = await giftCode(c);
  if (!code) return c.json({ error: "unknown" }, 401);
  const deny = (error: string) => c.json({ error }, 403, { "X-Gift-Refusal": error });
  const error = refusal(code, Date.now());
  if (error) return deny(error);
  const encrypted = code.keys[provider];
  // その会社のキーを預けていない
  if (encrypted === undefined) return deny("unsupported");
  const { success } = await c.env.GIFT_RATE.limit({ key: code.id });
  if (!success) return c.json({ error: "rate" }, 429);
  return { code, key: await decrypt(encrypted, c.env.SHARE_KEY) };
}

/** JSON の本文。読めなければ undefined（400 にする） */
async function json<T = Record<string, unknown>>(c: Ctx): Promise<T | undefined> {
  return parsed<T>(await c.req.text());
}

function parsed<T>(text: string): T | undefined {
  try {
    const body: unknown = JSON.parse(text);
    return body !== null && typeof body === "object" ? (body as T) : undefined;
  } catch {
    return undefined;
  }
}

interface CodeFields {
  memo?: unknown;
  expiresAt?: unknown;
  dailyLimitUsd?: unknown;
}

/** メモ・期限・1 日の上限を確かめる。省いた項目は undefined、期限なしと既定の上限は null */
function codeFields(body: CodeFields) {
  const { memo, expiresAt, dailyLimitUsd } = body;
  if (memo !== undefined && (typeof memo !== "string" || memo.length > MAX_MEMO)) return undefined;
  if (expiresAt !== undefined && expiresAt !== null && !Number.isSafeInteger(expiresAt)) {
    return undefined;
  }
  if (
    dailyLimitUsd !== undefined &&
    dailyLimitUsd !== null &&
    !(typeof dailyLimitUsd === "number" && Number.isFinite(dailyLimitUsd) && dailyLimitUsd > 0)
  ) {
    return undefined;
  }
  return {
    memo: memo as string | undefined,
    expiresAt: expiresAt as number | null | undefined,
    dailyLimitUsd: dailyLimitUsd as number | null | undefined,
  };
}

export default app;
