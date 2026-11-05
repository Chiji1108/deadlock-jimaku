// コード・合言葉・キーの扱い。どれもそのものは保存しない（キーは暗号化、ほかはハッシュ）

/** Crockford Base32。O と 0、I・L と 1 を同じに扱えるよう、まぎらわしい文字を使わない */
const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/** `K7M2-9QXT-4HNB`（60 ビット） */
export function newCode(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(12));
  const chars = [...bytes].map((b) => ALPHABET[b % 32]).join("");
  return chars.match(/.{4}/g)!.join("-");
}

/** 手で打ったコードを揃える（区切り・大文字小文字・まぎらわしい文字） */
export function normalizeCode(code: string): string {
  return code
    .toUpperCase()
    .replace(/[\s-]/g, "")
    .replace(/O/g, "0")
    .replace(/[IL]/g, "1");
}

/** おすそ分けする人の合言葉。人ごとに決まった値にして、どの PC でも同じものを使う */
export async function issuerToken(id: string, secret: string): Promise<string> {
  const raw = Uint8Array.from(atob(secret), (c) => c.charCodeAt(0));
  const key = await crypto.subtle.importKey("raw", raw, { name: "HMAC", hash: "SHA-256" }, false, [
    "sign",
  ]);
  const mac = await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(`issuer:${id}`));
  return base64url(new Uint8Array(mac));
}

export function newId(): string {
  return base64url(crypto.getRandomValues(new Uint8Array(12)));
}

export async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return base64url(new Uint8Array(digest));
}

/** `secret` は 32 バイトの鍵の base64（Worker の秘密の設定 SHARE_KEY） */
export async function encrypt(plain: string, secret: string): Promise<string> {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const data = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv },
    await aesKey(secret),
    new TextEncoder().encode(plain),
  );
  return btoa(String.fromCharCode(...iv, ...new Uint8Array(data)));
}

export async function decrypt(sealed: string, secret: string): Promise<string> {
  const bytes = Uint8Array.from(atob(sealed), (c) => c.charCodeAt(0));
  const plain = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv: bytes.slice(0, 12) },
    await aesKey(secret),
    bytes.slice(12),
  );
  return new TextDecoder().decode(plain);
}

function aesKey(secret: string): Promise<CryptoKey> {
  const raw = Uint8Array.from(atob(secret), (c) => c.charCodeAt(0));
  return crypto.subtle.importKey("raw", raw, "AES-GCM", false, ["encrypt", "decrypt"]);
}

function base64url(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}
