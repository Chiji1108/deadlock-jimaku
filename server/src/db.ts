// D1 の読み書き（migrations/）

import { normalizeCode, sha256 } from "./secrets";

/** 期限を過ぎても、始めていた配信はこの間だけ続けられる（期限は字幕を始めるときに確かめる） */
export const GRACE_MS = 24 * 60 * 60 * 1000;

/** コードの 1 日の上限（ドル）の既定。1 人が 24 時間使い続けても届かない値（DESIGN.md 17.1） */
export const DEFAULT_DAILY_LIMIT_USD = 40;

export interface Issuer {
  id: string;
  encrypted_key: string;
}

export interface Code {
  id: string;
  issuer_id: string;
  expires_at: number | null;
  revoked: number;
  /** NULL は既定（DEFAULT_DAILY_LIMIT_USD） */
  daily_limit_usd: number | null;
  /** day_usd を数えている日 */
  day: string | null;
  day_usd: number;
  /** おすそ分けした人の暗号化したキー */
  encrypted_key: string;
}

const SELECT_CODE = `SELECT c.id, c.issuer_id, c.expires_at, c.revoked,
    c.daily_limit_usd, c.day, c.day_usd, i.encrypted_key
  FROM codes c JOIN issuers i ON i.id = c.issuer_id`;

export async function issuerByToken(db: D1Database, token: string): Promise<Issuer | null> {
  return db
    .prepare("SELECT id, encrypted_key FROM issuers WHERE admin_token_hash = ?")
    .bind(await sha256(token))
    .first<Issuer>();
}

export async function codeByValue(db: D1Database, code: string): Promise<Code | null> {
  return db
    .prepare(`${SELECT_CODE} WHERE c.code_hash = ?`)
    .bind(await sha256(normalizeCode(code)))
    .first<Code>();
}

export async function codeById(db: D1Database, id: string): Promise<Code | null> {
  return db
    .prepare(`${SELECT_CODE} WHERE c.id = ?`)
    .bind(id)
    .first<Code>();
}

/** 中継しない理由。期限は猶予（GRACE_MS）を足して見る。使えれば undefined */
export function refusal(code: Code, now: number): "revoked" | "expired" | "limit" | undefined {
  if (code.revoked) return "revoked";
  if (code.expires_at !== null && now >= code.expires_at + GRACE_MS) return "expired";
  if (overLimit(code, now)) return "limit";
  return undefined;
}

/** 今日（UTC）の上限に達したか */
export function overLimit(code: Code, now: number): boolean {
  const spent = code.day === day(now) ? code.day_usd : 0;
  return spent >= (code.daily_limit_usd ?? DEFAULT_DAILY_LIMIT_USD);
}

/** コードで使われた額を足す（月の集計と今日の分） */
export async function addUsage(db: D1Database, codeId: string, usd: number, now: number) {
  if (usd <= 0) return;
  await db.batch([
    db
      .prepare(
        `INSERT INTO code_usage (code_id, month, usd) VALUES (?, ?, ?)
         ON CONFLICT (code_id, month) DO UPDATE SET usd = usd + excluded.usd`,
      )
      .bind(codeId, month(now), usd),
    db
      .prepare(
        "UPDATE codes SET day_usd = CASE WHEN day = ?1 THEN day_usd + ?2 ELSE ?2 END, day = ?1 WHERE id = ?3",
      )
      .bind(day(now), usd, codeId),
  ]);
}

export function month(now: number): string {
  return new Date(now).toISOString().slice(0, 7);
}

export function day(now: number): string {
  return new Date(now).toISOString().slice(0, 10);
}
