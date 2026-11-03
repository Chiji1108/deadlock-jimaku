// D1 の読み書き（migrations/0001_share.sql）

import { normalizeCode, sha256 } from "./secrets";

/** 期限を過ぎても、始めていた配信はこの間だけ続けられる（期限は字幕を始めるときに確かめる） */
export const GRACE_MS = 24 * 60 * 60 * 1000;

export interface Issuer {
  id: string;
  encrypted_key: string;
}

export interface Code {
  id: string;
  issuer_id: string;
  expires_at: number | null;
  revoked: number;
}

export async function issuerByToken(db: D1Database, token: string): Promise<Issuer | null> {
  return db
    .prepare("SELECT id, encrypted_key FROM issuers WHERE admin_token_hash = ?")
    .bind(await sha256(token))
    .first<Issuer>();
}

export async function codeByValue(db: D1Database, code: string): Promise<Code | null> {
  return db
    .prepare("SELECT id, issuer_id, expires_at, revoked FROM codes WHERE code_hash = ?")
    .bind(await sha256(normalizeCode(code)))
    .first<Code>();
}

/** 中継に使えるか。期限は猶予（GRACE_MS）を足して見る */
export function usable(code: Code, now: number): boolean {
  return !code.revoked && (code.expires_at === null || now < code.expires_at + GRACE_MS);
}

/** コードで使われた額を足す */
export async function addUsage(db: D1Database, codeId: string, usd: number, now: number) {
  if (usd <= 0) return;
  await db
    .prepare(
      `INSERT INTO code_usage (code_id, month, usd) VALUES (?, ?, ?)
       ON CONFLICT (code_id, month) DO UPDATE SET usd = usd + excluded.usd`,
    )
    .bind(codeId, month(now), usd)
    .run();
}

export function month(now: number): string {
  return new Date(now).toISOString().slice(0, 7);
}
