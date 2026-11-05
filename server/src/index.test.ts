import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { getPlatformProxy } from "wrangler";
import schema from "../migrations/0001_share.sql?raw";
import { app, type Env } from "./index";
import { openai, validKey } from "./openai";
import { appendSeconds, tokensUsd, wavSeconds } from "./price";
import { normalizeCode } from "./secrets";

vi.mock("./openai", () => ({ openai: vi.fn(), validKey: vi.fn() }));

const KEY = "sk-issuer";
let proxy: Awaited<ReturnType<typeof getPlatformProxy<{ DB: D1Database }>>>;
let env: Env;
let pending: Promise<unknown>[] = [];
const ctx = { waitUntil: (p: Promise<unknown>) => pending.push(p), passThroughOnException() {}, props: {} };

/** 手元の D1 で API を呼ぶ。後で書く分（waitUntil）も待つ */
async function call(path: string, init: RequestInit & { token?: string } = {}) {
  const headers = new Headers(init.headers);
  if (init.token) headers.set("Authorization", `Bearer ${init.token}`);
  const res = await app.request(`/v1${path}`, { ...init, headers }, env, ctx as never);
  await Promise.all(pending);
  pending = [];
  return res;
}

const json = (body: unknown) => ({
  body: JSON.stringify(body),
  headers: { "Content-Type": "application/json" },
});

beforeAll(async () => {
  proxy = await getPlatformProxy<{ DB: D1Database }>({ persist: false });
  env = { DB: proxy.env.DB, SHARE_KEY: btoa(String.fromCharCode(...new Uint8Array(32).fill(7))) };
  const sql = schema
    .replace(/--.*$/gm, "")
    .split(";")
    .map((s) => s.trim())
    .filter(Boolean);
  for (const statement of sql) await env.DB.prepare(statement).run();
});

afterAll(() => proxy.dispose());

beforeEach(async () => {
  for (const table of ["code_usage", "codes", "issuers"]) {
    await env.DB.prepare(`DELETE FROM ${table}`).run();
  }
  vi.mocked(validKey).mockImplementation(async (key) => key.startsWith("sk-"));
  vi.mocked(openai).mockReset();
});

async function share(key = KEY): Promise<string> {
  const res = await call("/share", { method: "POST", ...json({ key }) });
  expect(res.status).toBe(200);
  return ((await res.json()) as { token: string }).token;
}

async function createCode(token: string, body: { memo?: string; expiresAt?: number | null } = {}) {
  const res = await call("/share/codes", { method: "POST", token, ...json(body) });
  expect(res.status).toBe(200);
  return (await res.json()) as { id: string; code: string };
}

async function listCodes(token: string) {
  const res = await call("/share/codes", { token });
  return ((await res.json()) as { codes: { id: string; usd: number; monthUsd: number; revoked: boolean; memo: string; expiresAt: number | null }[] }).codes;
}

describe("share", () => {
  it("needs a working key", async () => {
    expect((await call("/share", { method: "POST", ...json({ key: "bad" }) })).status).toBe(400);
  });

  it("shares the same codes on every PC with the same key", async () => {
    const first = await share();
    await createCode(first, { memo: "友達" });
    const second = await share();
    expect(second).toBe(first);
    expect((await listCodes(first)).map((c) => c.memo)).toEqual(["友達"]);
    expect((await listCodes(second)).map((c) => c.memo)).toEqual(["友達"]);
  });

  it("gives a different token to a different key, and a new one after stopping", async () => {
    const mine = await share();
    expect(await share("sk-other")).not.toBe(mine);
    await call("/share", { method: "DELETE", token: mine });
    expect((await call("/share/codes", { token: mine })).status).toBe(401);
    expect(await share()).not.toBe(mine);
  });

  it("codes are random, shown once, and accept loose typing", async () => {
    const token = await share();
    const { code } = await createCode(token);
    expect(code).toMatch(/^[0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}$/);
    expect(normalizeCode("ko1l-")).toBe("K011");
    const loose = code.toLowerCase().replace(/-/g, " ");
    expect((await call("/gift", { token: loose })).status).toBe(200);
  });

  it("refuses to switch to a key already shared by someone else", async () => {
    const mine = await share();
    await share("sk-other");
    const res = await call("/share/key", { method: "PUT", token: mine, ...json({ key: "sk-other" }) });
    expect(res.status).toBe(409);
  });

  it("only the issuer can change a code", async () => {
    const mine = await share();
    const { id } = await createCode(mine);
    const other = await share("sk-other");
    expect(
      (await call(`/share/codes/${id}`, { method: "PATCH", token: other, ...json({ revoked: true }) })).status,
    ).toBe(404);
  });

  it("rejects a bad expiry or memo", async () => {
    const token = await share();
    for (const body of [{ expiresAt: "tomorrow" }, { memo: "x".repeat(101) }]) {
      expect((await call("/share/codes", { method: "POST", token, ...json(body) })).status).toBe(400);
    }
  });

  it("disabling removes the key and the codes", async () => {
    const token = await share();
    const { code } = await createCode(token);
    expect((await call("/share", { method: "DELETE", token })).status).toBe(204);
    expect((await call("/gift", { token: code })).status).toBe(401);
  });
});

describe("gift", () => {
  it("tells the expiry, and refuses after it (but relays a while longer for a running stream)", async () => {
    const token = await share();
    const future = Date.now() + 60_000;
    const { code, id } = await createCode(token, { expiresAt: future });
    expect(await (await call("/gift", { token: code })).json()).toEqual({ expiresAt: future });

    // 期限を過ぎた直後: 始められないが、中継はする
    const past = Date.now() - 60_000;
    await call(`/share/codes/${id}`, { method: "PATCH", token, ...json({ expiresAt: past }) });
    expect((await call("/gift", { token: code })).status).toBe(403);
    vi.mocked(openai).mockResolvedValue(Response.json({ usage: {} }));
    const body = json({ model: "gpt-6.1-terra", input: "x" });
    expect((await call("/openai/responses", { method: "POST", token: code, ...body })).status).toBe(200);

    // 猶予も過ぎたら中継しない
    await call(`/share/codes/${id}`, {
      method: "PATCH",
      token,
      ...json({ expiresAt: Date.now() - 25 * 60 * 60 * 1000 }),
    });
    expect((await call("/openai/responses", { method: "POST", token: code, ...body })).status).toBe(403);
  });

  it("stops at once when revoked, and can be resumed", async () => {
    const token = await share();
    const { code, id } = await createCode(token, { expiresAt: null });
    const revoke = (revoked: boolean) =>
      call(`/share/codes/${id}`, { method: "PATCH", token, ...json({ revoked }) });
    await revoke(true);
    expect(await (await call("/gift", { token: code })).json()).toEqual({ error: "revoked" });
    await revoke(false);
    expect(await (await call("/gift", { token: code })).json()).toEqual({ expiresAt: null });
  });

  it("unknown codes are refused", async () => {
    expect((await call("/gift", { token: "0000-0000-0000" })).status).toBe(401);
    expect((await call("/gift")).status).toBe(401);
  });
});

describe("relay", () => {
  it("sends responses with the issuer's key, unstored, and counts the cost", async () => {
    const token = await share();
    const { code } = await createCode(token);
    vi.mocked(openai).mockResolvedValue(
      Response.json({
        output: [],
        usage: { input_tokens: 10_000, input_tokens_details: { cached_tokens: 8_000 }, output_tokens: 1_000 },
      }),
    );
    const res = await call("/openai/responses", {
      method: "POST",
      token: code,
      ...json({ model: "gpt-6.1-terra", input: "ヘイズ", store: true }),
    });
    expect(res.status).toBe(200);
    const [path, key, init] = vi.mocked(openai).mock.calls[0]!;
    expect([path, key]).toEqual(["/v1/responses", KEY]);
    expect(JSON.parse(init!.body as string)).toMatchObject({ store: false });
    const [listed] = await listCodes(token);
    // 2,000 × $1 + 8,000 × $0.05 + 1,000 × $5（100 万トークンあたり）
    expect(listed!.usd).toBeCloseTo(0.0074);
    expect(listed!.monthUsd).toBeCloseTo(0.0074);
  });

  it("refuses models without a price", async () => {
    const token = await share();
    const { code } = await createCode(token);
    const res = await call("/openai/responses", { method: "POST", token: code, ...json({ model: "gpt-9-ultra" }) });
    expect(res.status).toBe(400);
    expect(openai).not.toHaveBeenCalled();
  });

  it("passes only the fields the app uses (no paid tools, no streaming)", async () => {
    const token = await share();
    const { code } = await createCode(token);
    vi.mocked(openai).mockResolvedValue(Response.json({ usage: {} }));
    const body = {
      model: "gpt-6.1-terra",
      instructions: "sys",
      input: "x",
      text: { format: { type: "json_schema" } },
      reasoning: { effort: "none" },
      stream: true,
      tools: [{ type: "web_search" }],
      background: true,
    };
    await call("/openai/responses", { method: "POST", token: code, ...json(body) });
    const sent = JSON.parse(vi.mocked(openai).mock.calls[0]![2]!.body as string);
    expect(Object.keys(sent).sort()).toEqual(["input", "instructions", "model", "reasoning", "store", "text"]);
  });

  it("refuses WAV that is not what the app sends", async () => {
    const token = await share();
    const { code } = await createCode(token);
    const form = new FormData();
    form.set("model", "gpt-transcribe");
    form.set("file", new File([wav(1, 48_000)], "segment.wav"));
    const res = await call("/openai/audio/transcriptions", { method: "POST", token: code, body: form });
    expect(res.status).toBe(400);
    expect(openai).not.toHaveBeenCalled();
  });

  it("answers broken bodies with 400", async () => {
    const token = await share();
    const broken = { body: "{", headers: { "Content-Type": "application/json" } };
    expect((await call("/share", { method: "POST", ...broken })).status).toBe(400);
    expect((await call("/share", { method: "POST", ...json(null) })).status).toBe(400);
    expect((await call("/share/key", { method: "PUT", token, ...broken })).status).toBe(400);
    const { code } = await createCode(token);
    expect((await call("/openai/responses", { method: "POST", token: code, ...broken })).status).toBe(400);
  });

  it("counts transcription by the length of the WAV", async () => {
    const token = await share();
    const { code } = await createCode(token);
    vi.mocked(openai).mockResolvedValue(Response.json({ text: "ヘイズ" }));
    const form = new FormData();
    form.set("model", "gpt-transcribe");
    form.set("file", new File([wav(60)], "segment.wav", { type: "audio/wav" }));
    const res = await call("/openai/audio/transcriptions", { method: "POST", token: code, body: form });
    expect(await res.json()).toEqual({ text: "ヘイズ" });
    expect((await listCodes(token))[0]!.usd).toBeCloseTo(0.0045);
  });

  it("does not count failed requests", async () => {
    const token = await share();
    const { code } = await createCode(token);
    vi.mocked(openai).mockResolvedValue(Response.json({ error: {} }, { status: 500 }));
    const body = json({ model: "gpt-6.1-terra", input: "x" });
    expect((await call("/openai/responses", { method: "POST", token: code, ...body })).status).toBe(500);
    expect((await listCodes(token))[0]!.usd).toBe(0);
  });
});

describe("price", () => {
  it("reads the length of a WAV and of realtime audio", () => {
    expect(wavSeconds(wav(2))).toBeCloseTo(2);
    expect(wavSeconds(new ArrayBuffer(8))).toBeUndefined();
    // 長さはヘッダーではなく実際のバイト数で数える
    const forged = wav(2);
    new DataView(forged).setUint32(40, 0, true);
    expect(wavSeconds(forged)).toBeCloseTo(2);
    // 途中で切れたヘッダーでも投げない
    expect(wavSeconds(wav(2).slice(0, 30))).toBeUndefined();
    // 100ms の 24kHz・16bit = 4,800 バイト
    expect(appendSeconds(btoa("\0".repeat(4_800)))).toBeCloseTo(0.1);
  });

  it("prices tokens with the cached part cheaper", () => {
    expect(tokensUsd("gpt-6.1-terra", { input: 1e6, cached: 1e6, output: 0 })).toBeCloseTo(0.05);
    expect(tokensUsd("gpt-transcribe", { input: 1, cached: 0, output: 0 })).toBeUndefined();
  });
});

/** `seconds` 秒の 16bit・モノラルの WAV（中身は無音） */
function wav(seconds: number, rate = 16_000): ArrayBuffer {
  const data = rate * 2 * seconds;
  const buf = new ArrayBuffer(44 + data);
  const v = new DataView(buf);
  const tag = (at: number, s: string) => [...s].forEach((ch, i) => v.setUint8(at + i, ch.charCodeAt(0)));
  tag(0, "RIFF");
  v.setUint32(4, 36 + data, true);
  tag(8, "WAVE");
  tag(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, rate, true);
  v.setUint32(28, rate * 2, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  tag(36, "data");
  v.setUint32(40, data, true);
  return buf;
}
