// API（api.jimaku.deadlock.jp。DESIGN.md 17.1）。パスは版（/v1）で区切る。
// おすそ分け: 自分の OpenAI キーを預けた人がコードを作り、コードを持つ人の通信を、そのキーで OpenAI に中継する
import { Hono, type Context } from "hono";
import {
  addUsage,
  codeByValue,
  dailyLimit,
  issuerByToken,
  month,
  overLimit,
  refusal,
  todayUsd,
  type Code,
} from "./db";
import { openai, validKey } from "./openai";
import { audioUsd, tokensOf, tokensUsd, wavSeconds } from "./price";
import { relay } from "./realtime";
import { decrypt, encrypt, issuerToken, newCode, newId, normalizeCode, sha256 } from "./secrets";

export interface Env {
  DB: D1Database;
  /** キーを暗号化する鍵（32 バイトの base64）。`wrangler secret put SHARE_KEY` */
  SHARE_KEY: string;
  /** コードごとの 1 分あたりの要求数の上限（値は wrangler.jsonc。DESIGN.md 17.1） */
  GIFT_RATE: RateLimit;
}

type Ctx = Context<{ Bindings: Env }>;

export const app = new Hono<{ Bindings: Env }>().basePath("/v1");

/** memo の長さの上限 */
const MAX_MEMO = 100;

/** Responses API に渡す項目。ほかの項目（有料のツールなど）は、おすそ分けした人の料金を数えられないので通さない */
const RESPONSES_FIELDS = ["model", "instructions", "input", "text", "reasoning", "max_output_tokens", "stream"];

/**
 * 1 回の出力の上限（推論の分も含む）。アプリは送らないので、ここで必ず付ける。切れると JSON が読めなくなるので、
 * いちばん長い辞書の読み（40 語で 3,000 トークンほど）の 10 倍にする（DESIGN.md 17.1）
 */
const MAX_OUTPUT_TOKENS = 32_000;

/** 本文の長さの上限（文字数）。いちばん長い用語の判定（辞書の全部を送る。32,000 文字ほど）の 8 倍 */
const MAX_RESPONSES_BODY = 256_000;

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

/** キーを預ける。同じキーならどの PC からでも同じ人になり、同じ合言葉とコードを使う */
app.post("/share", async (c) => {
  const key = (await json<{ key?: unknown }>(c))?.key;
  if (typeof key !== "string" || !key) return c.json({ error: "noKey" }, 400);
  if (!(await validKey(key))) return c.json({ error: "invalidKey" }, 400);
  const keyHash = await sha256(key);
  const token = await issuerToken(keyHash, c.env.SHARE_KEY);
  await c.env.DB.prepare(
    `INSERT INTO issuers (id, admin_token_hash, key_hash, encrypted_key, created_at) VALUES (?, ?, ?, ?, ?)
     ON CONFLICT (key_hash) DO UPDATE SET admin_token_hash = excluded.admin_token_hash, encrypted_key = excluded.encrypted_key`,
  )
    .bind(newId(), await sha256(token), keyHash, await encrypt(key, c.env.SHARE_KEY), Date.now())
    .run();
  return c.json({ token });
});

/** キーを替えたとき。コードはそのまま、新しいキーで動く。合言葉も新しくなり、古いキーのものは使えなくなる */
app.put("/share/key", async (c) => {
  const issuer = await admin(c);
  if (!issuer) return c.json({ error: "unauthorized" }, 401);
  const key = (await json<{ key?: unknown }>(c))?.key;
  if (typeof key !== "string" || !key || !(await validKey(key))) {
    return c.json({ error: "invalidKey" }, 400);
  }
  const keyHash = await sha256(key);
  const other = await c.env.DB.prepare("SELECT id FROM issuers WHERE key_hash = ? AND id != ?")
    .bind(keyHash, issuer.id)
    .first();
  // そのキーで別の人としておすそ分けを有効にしている。片方を無効にしてもらう
  if (other) return c.json({ error: "keyInUse" }, 409);
  const token = await issuerToken(keyHash, c.env.SHARE_KEY);
  await c.env.DB.prepare(
    "UPDATE issuers SET key_hash = ?, encrypted_key = ?, admin_token_hash = ? WHERE id = ?",
  )
    .bind(keyHash, await encrypt(key, c.env.SHARE_KEY), await sha256(token), issuer.id)
    .run();
  return c.json({ token });
});

/** おすそ分けをやめる。預けたキーとコードを消す */
app.delete("/share", async (c) => {
  const issuer = await admin(c);
  if (!issuer) return c.json({ error: "unauthorized" }, 401);
  await c.env.DB.prepare("DELETE FROM issuers WHERE id = ?").bind(issuer.id).run();
  return c.body(null, 204);
});

app.get("/share/codes", async (c) => {
  const issuer = await admin(c);
  if (!issuer) return c.json({ error: "unauthorized" }, 401);
  const now = Date.now();
  const { results } = await c.env.DB.prepare(
    `SELECT c.id, c.memo, c.expires_at AS expiresAt, c.revoked, c.created_at AS createdAt,
            c.daily_limit_usd, c.day, c.day_usd,
            COALESCE(SUM(u.usd), 0) AS usd,
            COALESCE(SUM(CASE WHEN u.month = ? THEN u.usd END), 0) AS monthUsd
     FROM codes c LEFT JOIN code_usage u ON u.code_id = c.id
     WHERE c.issuer_id = ? GROUP BY c.id ORDER BY c.created_at DESC`,
  )
    .bind(month(now), issuer.id)
    .all<{ revoked: number } & Pick<Code, "daily_limit_usd" | "day" | "day_usd">>();
  // dailyLimitUsd は決めた額（null は既定）、effectiveLimitUsd は実際に効く額
  const codes = results.map(({ daily_limit_usd, day, day_usd, ...r }) => ({
    ...r,
    revoked: !!r.revoked,
    dayUsd: todayUsd({ day, day_usd }, now),
    dailyLimitUsd: daily_limit_usd,
    effectiveLimitUsd: dailyLimit({ daily_limit_usd }),
  }));
  return c.json({ codes });
});

/** コードを作る。コードそのものはこの応答でだけ返す（保存するのはハッシュ） */
app.post("/share/codes", async (c) => {
  const issuer = await admin(c);
  if (!issuer) return c.json({ error: "unauthorized" }, 401);
  const fields = codeFields((await json(c)) ?? {});
  if (!fields) return c.json({ error: "invalid" }, 400);
  const id = newId();
  const code = newCode();
  await c.env.DB.prepare(
    `INSERT INTO codes (id, code_hash, issuer_id, memo, expires_at, daily_limit_usd, created_at)
     VALUES (?, ?, ?, ?, ?, ?, ?)`,
  )
    .bind(
      id,
      await sha256(normalizeCode(code)),
      issuer.id,
      fields.memo ?? "",
      fields.expiresAt ?? null,
      fields.dailyLimitUsd ?? null,
      Date.now(),
    )
    .run();
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
  const sets: string[] = [];
  const values: unknown[] = [];
  if (fields.memo !== undefined) sets.push("memo = ?"), values.push(fields.memo);
  if (fields.expiresAt !== undefined) sets.push("expires_at = ?"), values.push(fields.expiresAt);
  if (fields.dailyLimitUsd !== undefined) sets.push("daily_limit_usd = ?"), values.push(fields.dailyLimitUsd);
  if (body.revoked !== undefined) sets.push("revoked = ?"), values.push(body.revoked ? 1 : 0);
  if (sets.length === 0) return c.body(null, 204);
  const { meta } = await c.env.DB.prepare(
    `UPDATE codes SET ${sets.join(", ")} WHERE id = ? AND issuer_id = ?`,
  )
    .bind(...values, c.req.param("id"), issuer.id)
    .run();
  return meta.changes ? c.body(null, 204) : c.json({ error: "notFound" }, 404);
});

// ---- コードを使う人 ----

/** コードの状態。アプリは止まっている字幕を始めるときと鍵を使い始めるときに、これで期限を確かめる（ここでは猶予を足さない） */
app.get("/gift", async (c) => {
  const code = await giftCode(c);
  if (!code) return c.json({ error: "unknown" }, 401);
  if (code.revoked) return c.json({ error: "revoked" }, 403);
  if (code.expires_at !== null && Date.now() >= code.expires_at) {
    return c.json({ error: "expired", expiresAt: code.expires_at }, 403);
  }
  if (overLimit(code, Date.now())) return c.json({ error: "limit" }, 403);
  return c.json({ expiresAt: code.expires_at });
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
      usage.then((u) => addUsage(c.env.DB, gift.code.id, tokensUsd(model, tokensOf(u)) ?? 0, Date.now())),
    );
    return new Response(client, { headers: { "Content-Type": "text/event-stream" } });
  }
  const answer = await res.text();
  if (res.ok) {
    const usage = (JSON.parse(answer) as { usage?: unknown }).usage;
    const usd = tokensUsd(model, tokensOf(usage)) ?? 0;
    c.executionCtx.waitUntil(addUsage(c.env.DB, gift.code.id, usd, Date.now()));
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
  const usd = seconds === undefined ? undefined : audioUsd(model, seconds);
  if (usd === undefined) return c.json({ error: "notAllowed" }, 400);
  const res = await openai("/v1/audio/transcriptions", gift.key, { method: "POST", body: form });
  if (res.ok) c.executionCtx.waitUntil(addUsage(c.env.DB, gift.code.id, usd, Date.now()));
  return new Response(res.body, {
    status: res.status,
    headers: { "Content-Type": res.headers.get("Content-Type") ?? "application/json" },
  });
});

app.get("/openai/realtime", async (c) => {
  if (c.req.header("Upgrade") !== "websocket") return c.json({ error: "websocket" }, 426);
  const gift = await giftKey(c);
  if (gift instanceof Response) return gift;
  return relay(c.env.DB, gift.code, gift.key, c.executionCtx);
});

// ---- 共通 ----

function bearer(c: Ctx): string | undefined {
  return c.req.header("Authorization")?.match(/^Bearer (.+)$/)?.[1];
}

async function admin(c: Ctx) {
  const token = bearer(c);
  return token ? issuerByToken(c.env.DB, token) : null;
}

async function giftCode(c: Ctx): Promise<Code | null> {
  const code = bearer(c);
  return code ? codeByValue(c.env.DB, code) : null;
}

/**
 * 中継に使えるコードと、おすそ分けした人のキー。使えなければ返す応答。
 * 理由はヘッダーにも載せる（WebSocket のクライアントは、断られたときの本文を読みきれるとは限らない）。
 * 429 は 403 のあとに見るので、止まる理由を隠さない
 */
async function giftKey(c: Ctx): Promise<{ code: Code; key: string } | Response> {
  const code = await giftCode(c);
  if (!code) return c.json({ error: "unknown" }, 401);
  const error = refusal(code, Date.now());
  if (error) return c.json({ error }, 403, { "X-Gift-Refusal": error });
  const { success } = await c.env.GIFT_RATE.limit({ key: code.id });
  if (!success) return c.json({ error: "rate" }, 429);
  return { code, key: await decrypt(code.encrypted_key, c.env.SHARE_KEY) };
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
