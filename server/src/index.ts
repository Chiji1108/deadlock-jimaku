// API（api.jimaku.deadlock.jp。DESIGN.md 17.1）。パスは版（/v1）で区切る。
// おすそ分け: 自分の OpenAI キーを預けた人がコードを作り、コードを持つ人の通信を、そのキーで OpenAI に中継する
import { Hono, type Context } from "hono";
import { addUsage, codeByValue, issuerByToken, month, usable, type Code } from "./db";
import { openai, validKey } from "./openai";
import { audioUsd, tokensOf, tokensUsd, wavSeconds } from "./price";
import { relay } from "./realtime";
import { decrypt, encrypt, issuerToken, newCode, newId, normalizeCode, sha256 } from "./secrets";

export interface Env {
  DB: D1Database;
  /** キーを暗号化する鍵（32 バイトの base64）。`wrangler secret put SHARE_KEY` */
  SHARE_KEY: string;
}

type Ctx = Context<{ Bindings: Env }>;

export const app = new Hono<{ Bindings: Env }>().basePath("/v1");

/** memo の長さの上限 */
const MAX_MEMO = 100;

/** Responses API に渡す項目。ほかの項目（有料のツールなど）は、おすそ分けした人の料金を数えられないので通さない */
const RESPONSES_FIELDS = ["model", "instructions", "input", "text", "reasoning", "max_output_tokens"];

// ---- おすそ分けする人（合言葉で見分ける） ----

/** キーを預ける。同じキーならどの PC からでも同じ人になり、同じ合言葉とコードを使う */
app.post("/share", async (c) => {
  const key = (await json<{ key?: unknown }>(c))?.key;
  if (typeof key !== "string" || !key) return c.json({ error: "noKey" }, 400);
  if (!(await validKey(key))) return c.json({ error: "invalidKey" }, 400);
  const keyHash = await sha256(key);
  const existing = await c.env.DB.prepare("SELECT id FROM issuers WHERE key_hash = ?")
    .bind(keyHash)
    .first<{ id: string }>();
  const id = existing?.id ?? newId();
  const token = await issuerToken(id, c.env.SHARE_KEY);
  await c.env.DB.prepare(
    `INSERT INTO issuers (id, admin_token_hash, key_hash, encrypted_key, created_at) VALUES (?, ?, ?, ?, ?)
     ON CONFLICT (key_hash) DO UPDATE SET admin_token_hash = excluded.admin_token_hash, encrypted_key = excluded.encrypted_key`,
  )
    .bind(id, await sha256(token), keyHash, await encrypt(key, c.env.SHARE_KEY), Date.now())
    .run();
  return c.json({ token });
});

/** キーを替えたとき。コードはそのまま、新しいキーで動く */
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
  await c.env.DB.prepare("UPDATE issuers SET key_hash = ?, encrypted_key = ? WHERE id = ?")
    .bind(keyHash, await encrypt(key, c.env.SHARE_KEY), issuer.id)
    .run();
  return c.body(null, 204);
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
  const { results } = await c.env.DB.prepare(
    `SELECT c.id, c.memo, c.expires_at AS expiresAt, c.revoked, c.created_at AS createdAt,
            COALESCE(SUM(u.usd), 0) AS usd,
            COALESCE(SUM(CASE WHEN u.month = ? THEN u.usd END), 0) AS monthUsd
     FROM codes c LEFT JOIN code_usage u ON u.code_id = c.id
     WHERE c.issuer_id = ? GROUP BY c.id ORDER BY c.created_at DESC`,
  )
    .bind(month(Date.now()), issuer.id)
    .all<{ revoked: number }>();
  return c.json({ codes: results.map((r) => ({ ...r, revoked: !!r.revoked })) });
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
    "INSERT INTO codes (id, code_hash, issuer_id, memo, expires_at, created_at) VALUES (?, ?, ?, ?, ?, ?)",
  )
    .bind(id, await sha256(normalizeCode(code)), issuer.id, fields.memo ?? "", fields.expiresAt ?? null, Date.now())
    .run();
  return c.json({ id, code });
});

/** メモ・期限を変える。止める（revoked）・止めたのを戻す */
app.patch("/share/codes/:id", async (c) => {
  const issuer = await admin(c);
  if (!issuer) return c.json({ error: "unauthorized" }, 401);
  const body = (await json<{ memo?: unknown; expiresAt?: unknown; revoked?: unknown }>(c)) ?? {};
  const fields = codeFields(body);
  if (!fields || (body.revoked !== undefined && typeof body.revoked !== "boolean")) {
    return c.json({ error: "invalid" }, 400);
  }
  const sets: string[] = [];
  const values: unknown[] = [];
  if (fields.memo !== undefined) sets.push("memo = ?"), values.push(fields.memo);
  if (fields.expiresAt !== undefined) sets.push("expires_at = ?"), values.push(fields.expiresAt);
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

/** コードの状態。アプリは字幕を始めるときにこれで期限を確かめる（ここでは猶予を足さない） */
app.get("/gift", async (c) => {
  const code = await giftCode(c);
  if (!code) return c.json({ error: "unknown" }, 401);
  if (code.revoked) return c.json({ error: "revoked" }, 403);
  if (code.expires_at !== null && Date.now() >= code.expires_at) {
    return c.json({ error: "expired", expiresAt: code.expires_at }, 403);
  }
  return c.json({ expiresAt: code.expires_at });
});

app.post("/openai/responses", async (c) => {
  const gift = await giftKey(c);
  if (gift instanceof Response) return gift;
  const body = (await json<Record<string, unknown>>(c)) ?? {};
  const model = body.model;
  // 料金は応答の usage で数えるので、ストリームにはしない（stream は通さない）
  if (typeof model !== "string" || tokensUsd(model, tokensOf(undefined)) === undefined) {
    return c.json({ error: "notAllowed" }, 400);
  }
  const forwarded = Object.fromEntries(RESPONSES_FIELDS.flatMap((k) => (k in body ? [[k, body[k]]] : [])));
  // 他人の字幕を通すので、OpenAI に応答を保存させない
  const res = await openai("/v1/responses", gift.key, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ...forwarded, store: false }),
  });
  const text = await res.text();
  if (res.ok) {
    const usage = (JSON.parse(text) as { usage?: unknown }).usage;
    const usd = tokensUsd(model, tokensOf(usage)) ?? 0;
    c.executionCtx.waitUntil(addUsage(c.env.DB, gift.code.id, usd, Date.now()));
  }
  return new Response(text, { status: res.status, headers: { "Content-Type": "application/json" } });
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

/** 中継に使えるコードと、おすそ分けした人のキー。使えなければ返す応答 */
async function giftKey(c: Ctx): Promise<{ code: Code; key: string } | Response> {
  const code = await giftCode(c);
  if (!code) return c.json({ error: "unknown" }, 401);
  if (!usable(code, Date.now())) return c.json({ error: code.revoked ? "revoked" : "expired" }, 403);
  return { code, key: await decrypt(code.encrypted_key, c.env.SHARE_KEY) };
}

/** JSON の本文。読めなければ undefined（400 にする） */
async function json<T = Record<string, unknown>>(c: Ctx): Promise<T | undefined> {
  const body: unknown = await c.req.json().catch(() => undefined);
  return body !== null && typeof body === "object" ? (body as T) : undefined;
}

/** メモと期限を確かめる。省いた項目は undefined、期限なしは null */
function codeFields(body: { memo?: unknown; expiresAt?: unknown }) {
  const { memo, expiresAt } = body;
  if (memo !== undefined && (typeof memo !== "string" || memo.length > MAX_MEMO)) return undefined;
  if (expiresAt !== undefined && expiresAt !== null && !Number.isSafeInteger(expiresAt)) {
    return undefined;
  }
  return { memo: memo as string | undefined, expiresAt: expiresAt as number | null | undefined };
}

export default app;
