// /v1（会社の形の中継）と /v2（仕事の口）が同じに使うもの: コードを読む、開発者のキーで外に送る、額を数える

import type { Context } from "hono";
import { keyFailed, reasonOf, type BetaEnv, type RelayKey } from "./beta";
import { codeByValue, type Code, type Db } from "./db";

export interface Env extends BetaEnv {
  DB: D1Database;
  /** 開発者のキーを暗号化する鍵（32 バイトの base64）。`wrangler secret put SHARE_KEY` */
  SHARE_KEY: string;
  /** コードごとの 1 分あたりの要求数の上限（値は wrangler.jsonc。DESIGN.md 17.2） */
  CODE_RATE: RateLimit;
}

export type App = { Bindings: Env; Variables: { db: Db } };
export type Ctx = Context<App>;

function bearer(c: Ctx): string | undefined {
  // 認証の方式の名前は、大文字と小文字を区別しない（アプリは Qwen の途中の字幕だけ小文字で送る）
  return c.req.header("Authorization")?.match(/^Bearer (.+)$/i)?.[1];
}

export async function bearerCode(c: Ctx): Promise<Code | null> {
  const code = bearer(c);
  return code ? codeByValue(c.var.db, code) : null;
}

export function parsed<T>(text: string): T | undefined {
  try {
    const body: unknown = JSON.parse(text);
    return body !== null && typeof body === "object" ? (body as T) : undefined;
  } catch {
    return undefined;
  }
}

/** 100 万トークンあたりの単価から、額を出す */
export const usdOf = (tokens: number, perMillion: number) => (tokens * perMillion) / 1e6;

/** OpenRouter の応答の料金。答えに入っていなければ undefined（見積もりで数える） */
export function openrouterUsd(usage: unknown): number | undefined {
  const cost = (usage as { cost?: unknown } | undefined)?.cost;
  return typeof cost === "number" ? cost : undefined;
}

/** キーのせいで断られたときの番号（ほかのキーなら通るかもしれない）。403 は入れない（本文が断られたときにも返る） */
const KEY_TROUBLE = [401, 402, 429];
/** キーを読めなかったときに覚える番号 */
const KEY_UNREADABLE = 0;

/**
 * 開発者のキーで外に送る。キーが断られたら（使えない、残高がない、回数の上限）、次のキーに回す。キーを読めないとき
 * （暗号化の鍵を替えた、行が壊れた）も同じ。断られたキーは覚えて、次からは後回しにする。
 * 返すのは最後の応答（どのキーでも送れなければ undefined）
 */
export async function sent(
  c: Ctx,
  keys: RelayKey[],
  send: (key: string) => Promise<Response>,
): Promise<Response | undefined> {
  const now = Date.now();
  let res: Response | undefined;
  for (const key of keys) {
    const secret = await key.key().catch(() => undefined);
    res = secret === undefined ? undefined : await send(secret).catch(() => undefined);
    // 読めなかったキーと、キーのせいで断られたとき（その番号）だけ、次に回す
    const refused = res && KEY_TROUBLE.includes(res.status) ? res.status : undefined;
    const trouble = secret === undefined ? KEY_UNREADABLE : refused;
    if (trouble === undefined) break;
    // 断られた応答は本文を読み切る（理由として覚える）。利用者には返さない
    const reason = await reasonOf(res);
    if (key.id) c.executionCtx.waitUntil(keyFailed(c.var.db, key.id, trouble, reason, now));
  }
  return res;
}
