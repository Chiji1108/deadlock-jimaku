// 管理画面の入口（DESIGN.md 17.3）。Cloudflare Access が前に立ち、通した人の JWT をヘッダーに付けてくる。
// ここでは、その JWT が本物か（Cloudflare の鍵の署名、この管理画面あてか、期限）を確かめる。ログインの仕組みは持たない

export interface AccessEnv {
  /** Cloudflare Zero Trust のチームの名前（`https://<名前>.cloudflareaccess.com`）。入れなければ、管理画面は閉じている */
  ACCESS_TEAM?: string;
  /** Access のアプリケーションの AUD（管理画面あての JWT だけを通す） */
  ACCESS_AUD?: string;
}

interface Jwk extends JsonWebKey {
  kid: string;
}

/** 署名を確かめる鍵（Cloudflare が入れ替えるので、1 時間ごとに取り直す） */
let cached: { team: string; at: number; keys: Jwk[] } | undefined;
const KEYS_TTL_MS = 60 * 60 * 1000;

/** 知らない鍵の名前（kid）が来たときに取り直す、いちばん短い間隔（鍵の入れ替えの直後に、1 時間待たせない） */
const KEYS_RETRY_MS = 60 * 1000;

async function signingKeys(team: string, now: number, kid: unknown): Promise<Jwk[]> {
  const fresh = cached && cached.team === team ? cached : undefined;
  const known = fresh?.keys.some((key) => key.kid === kid);
  const age = fresh ? now - fresh.at : Infinity;
  if (fresh && (known ? age < KEYS_TTL_MS : age < KEYS_RETRY_MS)) return fresh.keys;
  const res = await fetch(`https://${team}.cloudflareaccess.com/cdn-cgi/access/certs`).catch(() => undefined);
  const body = res?.ok ? ((await res.json().catch(() => undefined)) as { keys?: unknown } | undefined) : undefined;
  if (!Array.isArray(body?.keys)) return fresh?.keys ?? [];
  cached = { team, at: now, keys: body.keys as Jwk[] };
  return cached.keys;
}

function decoded(part: string): Uint8Array {
  const base64 = part.replace(/-/g, "+").replace(/_/g, "/");
  return Uint8Array.from(atob(base64.padEnd(Math.ceil(base64.length / 4) * 4, "=")), (c) => c.charCodeAt(0));
}

function json(part: string): Record<string, unknown> | undefined {
  try {
    const value: unknown = JSON.parse(new TextDecoder().decode(decoded(part)));
    return value !== null && typeof value === "object" ? (value as Record<string, unknown>) : undefined;
  } catch {
    return undefined;
  }
}

/** Access を通った人のメールアドレス。通っていなければ undefined（設定がなければ、誰も通さない） */
export async function accessEmail(request: Request, env: AccessEnv, now = Date.now()): Promise<string | undefined> {
  const { ACCESS_TEAM: team, ACCESS_AUD: aud } = env;
  const token = request.headers.get("Cf-Access-Jwt-Assertion");
  if (!team || !aud || !token) return undefined;
  const [head, body, signature, ...rest] = token.split(".");
  if (!head || !body || !signature || rest.length > 0) return undefined;
  const header = json(head);
  const claims = json(body);
  if (!header || !claims || header.alg !== "RS256") return undefined;
  const audiences = Array.isArray(claims.aud) ? claims.aud : [claims.aud];
  const valid =
    audiences.includes(aud) &&
    claims.iss === `https://${team}.cloudflareaccess.com` &&
    typeof claims.exp === "number" &&
    claims.exp * 1000 > now &&
    typeof claims.email === "string";
  if (!valid) return undefined;
  const jwk = (await signingKeys(team, now, header.kid)).find((key) => key.kid === header.kid);
  if (!jwk) return undefined;
  const key = await crypto.subtle.importKey("jwk", jwk, { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" }, false, [
    "verify",
  ]);
  const signed = new TextEncoder().encode(`${head}.${body}`);
  const ok = await crypto.subtle.verify("RSASSA-PKCS1-v1_5", key, decoded(signature), signed);
  return ok ? (claims.email as string) : undefined;
}
