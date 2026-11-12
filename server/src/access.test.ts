import { afterEach, expect, it, vi } from "vitest";
import { accessEmail } from "./access";

const TEAM = "https://team.cloudflareaccess.com";
const AUD = "admin-aud";
const env = { TEAM_DOMAIN: TEAM, POLICY_AUD: AUD };
const NOW = 1_800_000_000_000;

const base64url = (bytes: ArrayBuffer | Uint8Array | string) =>
  btoa(typeof bytes === "string" ? bytes : String.fromCharCode(...new Uint8Array(bytes)))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");

/** Cloudflare の鍵の代わりの鍵と、それで署名する関数。`fetch` は、その公開鍵を返す */
async function issuer(kid = "k1") {
  const algorithm = { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" };
  const pair = (await crypto.subtle.generateKey(algorithm, true, ["sign", "verify"])) as CryptoKeyPair;
  const jwk = { ...(await crypto.subtle.exportKey("jwk", pair.publicKey)), kid };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string) => {
      expect(url).toBe(`${TEAM}/cdn-cgi/access/certs`);
      return Response.json({ keys: [jwk] });
    }),
  );
  return async (claims: Record<string, unknown>, header: Record<string, unknown> = { alg: "RS256", kid }) => {
    const signed = `${base64url(JSON.stringify(header))}.${base64url(JSON.stringify(claims))}`;
    const signature = await crypto.subtle.sign("RSASSA-PKCS1-v1_5", pair.privateKey, new TextEncoder().encode(signed));
    return `${signed}.${base64url(signature)}`;
  };
}

const request = (token?: string) =>
  new Request("https://admin.example/api/summary", { headers: token ? { "Cf-Access-Jwt-Assertion": token } : {} });

const claims = {
  aud: [AUD],
  iss: TEAM,
  exp: NOW / 1000 + 60,
  email: "dev@example.com",
};

afterEach(() => vi.unstubAllGlobals());

it("accepts only a token that Cloudflare signed for this admin page, before it expires", async () => {
  const sign = await issuer();
  const email = (token?: string) => accessEmail(request(token), env, NOW);
  expect(await email(await sign(claims))).toBe("dev@example.com");

  // ヘッダーがない、ほかのアプリケーションあて、ほかのチーム、期限切れ
  expect(await email()).toBeUndefined();
  expect(await email(await sign({ ...claims, aud: ["other-app"] }))).toBeUndefined();
  expect(await email(await sign({ ...claims, iss: "https://evil.cloudflareaccess.com" }))).toBeUndefined();
  expect(await email(await sign({ ...claims, exp: NOW / 1000 - 1 }))).toBeUndefined();
  // 署名なしの形式（alg: none）と、中身を書き換えたもの
  const [head, , signature] = (await sign(claims)).split(".");
  const forged = base64url(JSON.stringify({ ...claims, email: "attacker@example.com" }));
  expect(await email(`${head}.${forged}.${signature}`)).toBeUndefined();
  expect(await email(await sign(claims, { alg: "none", kid: "k1" }))).toBeUndefined();
  // 同じ名前（kid）の、ほかの鍵で署名したもの
  const stranger = await issuer();
  expect(await email(await stranger(claims))).toBeUndefined();
});

it("lets nobody in until the team and the application are configured", async () => {
  const sign = await issuer();
  const token = await sign(claims);
  expect(await accessEmail(request(token), {}, NOW)).toBeUndefined();
  expect(await accessEmail(request(token), { TEAM_DOMAIN: TEAM }, NOW)).toBeUndefined();
});
