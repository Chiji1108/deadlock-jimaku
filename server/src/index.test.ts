import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { getPlatformProxy } from "wrangler";
import { accessEmail } from "./access";
import { admin, type AdminEnv } from "./admin";
import { network } from "./beta";
import { addUsage, codeByValue, day, DEFAULT_DAILY_LIMIT_USD, open, type Db } from "./db";
import { app, type Env } from "./index";
import { openai, validKey } from "./openai";
import { appendSeconds, tokensUsd, wavSeconds } from "./price";
import { relay } from "./realtime";
import { encrypt, normalizeCode } from "./secrets";
import { openrouter, qwen, qwenLive } from "./upstream";
import { validXaiKey, xai } from "./xai";

vi.mock("./upstream", () => ({ openrouter: vi.fn(), qwen: vi.fn(), qwenLive: vi.fn() }));
vi.mock("./access", () => ({ accessEmail: vi.fn() }));
vi.mock("./openai", () => ({ openai: vi.fn(), validKey: vi.fn() }));
vi.mock("./xai", () => ({ xai: vi.fn(), validXaiKey: vi.fn() }));

const KEY = "sk-issuer";
let proxy: Awaited<ReturnType<typeof getPlatformProxy<{ DB: D1Database }>>>;
let env: Env;
let db: Db;
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

const rate = vi.fn<RateLimit["limit"]>();

beforeAll(async () => {
  proxy = await getPlatformProxy<{ DB: D1Database }>({ persist: false });
  env = {
    DB: proxy.env.DB,
    SHARE_KEY: btoa(String.fromCharCode(...new Uint8Array(32).fill(7))),
    GIFT_RATE: { limit: rate },
  };
  db = open(env.DB);
  // 本番と同じく、migrations/ の SQL を番号の順に当てる
  const migrations = import.meta.glob<string>("../migrations/*.sql", { query: "?raw", import: "default", eager: true });
  const sql = Object.keys(migrations)
    .sort()
    .map((name) => migrations[name])
    .join("\n")
    .replace(/--.*$/gm, "")
    .split(";")
    .map((s) => s.trim())
    .filter(Boolean);
  for (const statement of sql) await env.DB.prepare(statement).run();
});

afterAll(() => proxy.dispose());

beforeEach(async () => {
  for (const table of ["code_usage", "codes", "issuer_keys", "issuers", "beta_grants", "beta_spend", "beta_keys", "beta_settings"]) {
    await env.DB.prepare(`DELETE FROM ${table}`).run();
  }
  vi.mocked(validKey).mockImplementation(async (key) => key.startsWith("sk-"));
  vi.mocked(openai).mockReset();
  vi.mocked(validXaiKey).mockImplementation(async (key) => key.startsWith("xai-"));
  vi.mocked(xai).mockReset();
  vi.mocked(accessEmail).mockReset();
  vi.mocked(accessEmail).mockResolvedValue("dev@example.com");
  vi.mocked(qwen).mockReset();
  vi.mocked(qwenLive).mockReset();
  vi.mocked(openrouter).mockReset();
  env.BETA_QWEN_KEY = "sk-beta-qwen";
  env.BETA_OPENROUTER_KEY = "sk-beta-openrouter";
  rate.mockReset();
  rate.mockResolvedValue({ success: true });
});

async function share(key = KEY, provider?: string): Promise<string> {
  const res = await call("/share", { method: "POST", ...json(provider ? { provider, key } : { key }) });
  expect(res.status).toBe(200);
  return ((await res.json()) as { token: string }).token;
}

const XAI_KEY = "xai-issuer";

/** その会社のキーを預ける・替える。新しい合言葉を返す */
async function deposit(token: string, provider: string, key: string) {
  const res = await call(`/share/keys/${provider}`, { method: "PUT", token, ...json({ key }) });
  return { status: res.status, token: res.ok ? ((await res.json()) as { token: string }).token : undefined };
}

async function deposited(token: string) {
  return ((await (await call("/share/codes", { token })).json()) as { providers: string[] }).providers;
}

async function createCode(
  token: string,
  body: { memo?: string; expiresAt?: number | null; dailyLimitUsd?: number | null } = {},
) {
  const res = await call("/share/codes", { method: "POST", token, ...json(body) });
  expect(res.status).toBe(200);
  return (await res.json()) as { id: string; code: string };
}

async function listCodes(token: string) {
  const res = await call("/share/codes", { token });
  return ((await res.json()) as {
    codes: {
      id: string;
      usd: number;
      monthUsd: number;
      dayUsd: number;
      dailyLimitUsd: number | null;
      effectiveLimitUsd: number;
      revoked: boolean;
      memo: string;
      expiresAt: number | null;
    }[];
  }).codes;
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

  it("gives a different token to a different key", async () => {
    expect(await share("sk-other")).not.toBe(await share());
  });

  it("stops on every PC, and starts empty when shared again", async () => {
    const token = await share();
    await createCode(token, { memo: "友達" });
    await call("/share", { method: "DELETE", token });
    expect((await call("/share/codes", { token })).status).toBe(401);
    expect(await listCodes(await share())).toEqual([]);
  });

  it("codes are random, shown once, and accept loose typing", async () => {
    const token = await share();
    const { code } = await createCode(token);
    expect(code).toMatch(/^[0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}$/);
    expect(normalizeCode("ko1l-")).toBe("K011");
    const loose = code.toLowerCase().replace(/-/g, " ");
    expect((await call("/gift", { token: loose })).status).toBe(200);
  });

  it("replaces the token when the key changes, keeping the codes", async () => {
    const old = await share();
    await createCode(old, { memo: "友達" });
    const res = await call("/share/key", { method: "PUT", token: old, ...json({ key: "sk-new" }) });
    expect(res.status).toBe(200);
    const { token } = (await res.json()) as { token: string };
    expect(token).not.toBe(old);
    expect((await call("/share/codes", { token: old })).status).toBe(401);
    expect((await listCodes(token)).map((c) => c.memo)).toEqual(["友達"]);
    expect(await share("sk-new")).toBe(token);
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
    expect(await (await call("/gift", { token: code })).json()).toEqual({ expiresAt: future, providers: ["openai"] });

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
    expect(await (await call("/gift", { token: code })).json()).toEqual({ expiresAt: null, providers: ["openai"] });
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

  it("relays decisions with only text choices, and counts the input", async () => {
    const token = await share();
    const { code } = await createCode(token);
    vi.mocked(openai).mockResolvedValue(
      Response.json({
        answers: [{ type: "choice", name: "c0", choice: "hero:13" }],
        usage: { input_tokens: 10_000, input_tokens_details: { cached_tokens: 0 }, output_tokens: 0 },
      }),
    );
    const question = {
      type: "choice",
      name: "c0",
      instructions: "What does ヘイズ mean?",
      choices: [
        { value: "hero:13", description: "Haze" },
        { value: "none", description: "None" },
      ],
    };
    const ask = (body: unknown) => call("/openai/decisions", { method: "POST", token: code, ...json(body) });
    const good = { model: "gpt-6-luna", input: "ヘイズ強い", questions: [question] };
    const res = await ask({ ...good, store: true });
    expect(await res.json()).toMatchObject({ answers: [{ choice: "hero:13" }] });
    const [path, key, init] = vi.mocked(openai).mock.calls[0]!;
    expect([path, key]).toEqual(["/v1/decisions", KEY]);
    expect(JSON.parse(init!.body as string)).toEqual(good);
    // 10,000 × $0.10（100 万トークンあたり）。出力は数えない
    expect((await listCodes(token))[0]!.usd).toBeCloseTo(0.001);

    // ほかのモデル、文字でない input、画像を渡せる問い、問いの数
    const image = { ...question, instructions: [{ type: "input_image", image_url: "https://example.com/a.png" }] };
    for (const bad of [
      { ...good, model: "gpt-6.1-terra" },
      { ...good, input: [{ role: "user", content: "x" }] },
      { ...good, questions: [image] },
      { ...good, questions: [{ ...question, type: "predicate" }] },
      { ...good, questions: [] },
      { ...good, questions: Array(41).fill(question) },
    ]) {
      expect((await ask(bad)).status).toBe(400);
    }
    expect((await ask({ ...good, input: "x".repeat(64_001) })).status).toBe(413);
    expect(openai).toHaveBeenCalledOnce();
  });

  it("passes a stream through and counts the cost from its last response", async () => {
    const token = await share();
    const { code } = await createCode(token);
    const usage = { input_tokens: 10_000, input_tokens_details: { cached_tokens: 8_000 }, output_tokens: 1_000 };
    const sse = [
      `data: ${JSON.stringify({ type: "response.output_text.delta", delta: "{" })}`,
      `data: ${JSON.stringify({ type: "response.completed", response: { usage } })}`,
      "data: [DONE]",
      "",
    ].join("\n\n");
    vi.mocked(openai).mockResolvedValue(new Response(sse));
    const res = await call("/openai/responses", {
      method: "POST",
      token: code,
      ...json({ model: "gpt-6.1-terra", input: "ヘイズ", stream: true }),
    });
    expect(res.headers.get("Content-Type")).toBe("text/event-stream");
    expect(await res.text()).toBe(sse);
    expect(JSON.parse(vi.mocked(openai).mock.calls[0]![2]!.body as string)).toMatchObject({ stream: true });
    const [listed] = await listCodes(token);
    expect(listed!.usd).toBeCloseTo(0.0074);
  });

  it("refuses models without a price", async () => {
    const token = await share();
    const { code } = await createCode(token);
    const res = await call("/openai/responses", { method: "POST", token: code, ...json({ model: "gpt-9-ultra" }) });
    expect(res.status).toBe(400);
    expect(openai).not.toHaveBeenCalled();
  });

  it("passes only the fields the app uses (no paid tools)", async () => {
    const token = await share();
    const { code } = await createCode(token);
    vi.mocked(openai).mockResolvedValue(Response.json({ usage: {} }));
    const body = {
      model: "gpt-6.1-terra",
      instructions: "sys",
      input: "x",
      text: { format: { type: "json_schema" } },
      reasoning: { effort: "none" },
      tools: [{ type: "web_search" }],
      background: true,
    };
    await call("/openai/responses", { method: "POST", token: code, ...json(body) });
    const sent = JSON.parse(vi.mocked(openai).mock.calls[0]![2]!.body as string);
    expect(Object.keys(sent).sort()).toEqual([
      "input",
      "instructions",
      "max_output_tokens",
      "model",
      "reasoning",
      "store",
      "text",
    ]);
  });

  it("caps the output, whether the app asks for a size or not", async () => {
    const token = await share();
    const { code } = await createCode(token);
    vi.mocked(openai).mockImplementation(async () => Response.json({ usage: {} }));
    const sent = async (max?: unknown) => {
      const body = { model: "gpt-6.1-terra", input: "x", max_output_tokens: max };
      await call("/openai/responses", { method: "POST", token: code, ...json(body) });
      return JSON.parse(vi.mocked(openai).mock.lastCall![2]!.body as string).max_output_tokens;
    };
    // 今のアプリも v0.2.13 も送らない
    expect(await sent()).toBe(32_000);
    expect(await sent(1_000_000)).toBe(32_000);
    expect(await sent(500)).toBe(500);
    expect(await sent("many")).toBe(32_000);
  });

  it("refuses oversized bodies and inputs that are not text", async () => {
    const token = await share();
    const { code } = await createCode(token);
    vi.mocked(openai).mockImplementation(async () => Response.json({ usage: {} }));
    const post = (body: unknown) => call("/openai/responses", { method: "POST", token: code, ...json(body) });
    const base = { model: "gpt-6.1-terra", input: "x" };
    expect((await post({ ...base, instructions: "x".repeat(256_000) })).status).toBe(413);
    expect((await post({ ...base, input: [{ type: "input_image" }] })).status).toBe(400);
    expect(openai).not.toHaveBeenCalled();
    // 用語の判定の指示（辞書の全部）の 2 倍でも通る
    expect((await post({ ...base, instructions: "用".repeat(100_000) })).status).toBe(200);
  });

  it("accepts the requests of v0.2.13 as they are", async () => {
    const token = await share();
    const { code } = await createCode(token);
    vi.mocked(openai).mockImplementation(async () => Response.json({ usage: {} }));
    // crates/core/src/llm.rs の body()（v0.2.13 から変わっていない）
    const body = {
      model: "gpt-6.1-terra",
      instructions: "You process live commentary",
      input: JSON.stringify({ context: [], transcript: "ヘイズ強い" }),
      text: {
        format: {
          type: "json_schema",
          name: "subtitle",
          strict: true,
          schema: { type: "object", additionalProperties: false, required: [], properties: {} },
        },
      },
      store: false,
      reasoning: { effort: "none" },
    };
    for (const stream of [undefined, true]) {
      const res = await call("/openai/responses", { method: "POST", token: code, ...json({ ...body, stream }) });
      expect(res.status).toBe(200);
    }
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

describe("keys", () => {
  it("deposits a key per provider, only for the issuer and with a working key", async () => {
    const token = await share();
    const { code } = await createCode(token);
    expect((await deposit("nobody", "xai", XAI_KEY)).status).toBe(401);
    expect((await deposit(token, "xai", "bad")).status).toBe(400);
    expect((await deposit(token, "other", XAI_KEY)).status).toBe(404);
    expect(await (await call("/gift", { token: code })).json()).toMatchObject({ providers: ["openai"] });

    expect(await deposit(token, "xai", XAI_KEY)).toEqual({ status: 200, token });
    expect(await deposited(token)).toEqual(["openai", "xai"]);
    expect(await (await call("/gift", { token: code })).json()).toMatchObject({ providers: ["openai", "xai"] });

    const res = await call("/share/keys/xai", { method: "DELETE", token });
    const { token: after } = (await res.json()) as { token: string };
    expect(after).not.toBe(token);
    expect((await call("/share/codes", { token })).status).toBe(401);
    expect(await deposited(after)).toEqual(["openai"]);
  });

  it("shares with an xAI key alone, which relays to xAI but not to OpenAI", async () => {
    const token = await share(XAI_KEY, "xai");
    const { code } = await createCode(token);
    expect(await (await call("/gift", { token: code })).json()).toMatchObject({ providers: ["xai"] });
    const res = await call("/openai/responses", { method: "POST", token: code, ...json({ model: "gpt-6.1-terra", input: "x" }) });
    expect(res.status).toBe(403);
    expect(res.headers.get("X-Gift-Refusal")).toBe("unsupported");
    expect(openai).not.toHaveBeenCalled();
  });

  it("is the same issuer and token from any of the deposited keys", async () => {
    const token = await share();
    await deposit(token, "xai", XAI_KEY);
    expect(await share(XAI_KEY, "xai")).toBe(token);
  });

  it("joins only an existing share", async () => {
    const join = (key: string, provider: string) =>
      call("/share", { method: "POST", ...json({ provider, key, join: true }) });
    expect((await join(XAI_KEY, "xai")).status).toBe(404);
    const token = await share(XAI_KEY, "xai");
    expect(await (await join(XAI_KEY, "xai")).json()).toEqual({ token });
  });

  it("replaces the token when a key is replaced, keeping the other keys", async () => {
    const old = await share();
    await deposit(old, "xai", XAI_KEY);
    const { token } = await deposit(old, "xai", "xai-new");
    expect(token).not.toBe(old);
    expect((await call("/share/codes", { token: old })).status).toBe(401);
    expect(await deposited(token!)).toEqual(["openai", "xai"]);
    expect(await share(XAI_KEY, "xai")).not.toBe(token);
  });

  it("keeps the xAI key when the OpenAI key changes", async () => {
    const old = await share();
    await deposit(old, "xai", XAI_KEY);
    const res = await call("/share/key", { method: "PUT", token: old, ...json({ key: "sk-new" }) });
    const { token } = (await res.json()) as { token: string };
    expect(await deposited(token)).toEqual(["openai", "xai"]);
  });

  it("refuses a key that someone else shares, and to remove the last key", async () => {
    const mine = await share();
    await share(XAI_KEY, "xai");
    expect((await deposit(mine, "xai", XAI_KEY)).status).toBe(409);
    expect((await call("/share/keys/openai", { method: "DELETE", token: mine })).status).toBe(409);
  });
});

describe("xai", () => {
  function sttForm(model = "grok-voice-transcribe-2.0") {
    const form = new FormData();
    form.set("model", model);
    form.set("language", "ja");
    form.append("keyterm", "ヘイズ");
    form.append("keyterm", "デッドロック");
    form.set("prompt", "not passed");
    form.set("file", new File([wav(36)], "segment.wav", { type: "audio/wav" }));
    return form;
  }

  it("refuses to relay when no xAI key is deposited", async () => {
    const token = await share();
    const { code } = await createCode(token);
    const res = await call("/xai/stt", { method: "POST", token: code, body: sttForm() });
    expect(res.status).toBe(403);
    expect(res.headers.get("X-Gift-Refusal")).toBe("unsupported");
    expect(xai).not.toHaveBeenCalled();
  });

  it("relays with the issuer's xAI key, only the app's fields with the file last, and counts the cost", async () => {
    const token = await share();
    await deposit(token, "xai", XAI_KEY);
    const { code } = await createCode(token);
    vi.mocked(xai).mockResolvedValue(Response.json({ text: "ヘイズ", duration: 36 }));
    const res = await call("/xai/stt", { method: "POST", token: code, body: sttForm() });
    expect(await res.json()).toMatchObject({ text: "ヘイズ" });
    const [path, key, init] = vi.mocked(xai).mock.calls[0]!;
    expect([path, key]).toEqual(["/v1/stt", XAI_KEY]);
    const sent = init!.body as FormData;
    expect([...sent.keys()]).toEqual(["model", "language", "keyterm", "keyterm", "file"]);
    expect((await listCodes(token))[0]!.usd).toBeCloseTo(0.001);
  });

  it("refuses OpenAI models on xAI, and Grok on OpenAI", async () => {
    const token = await share();
    await deposit(token, "xai", XAI_KEY);
    const { code } = await createCode(token);
    const res = await call("/xai/stt", { method: "POST", token: code, body: sttForm("gpt-transcribe") });
    expect(res.status).toBe(400);
    const form = sttForm();
    const openaiRes = await call("/openai/audio/transcriptions", { method: "POST", token: code, body: form });
    expect(openaiRes.status).toBe(400);
    expect(xai).not.toHaveBeenCalled();
    expect(openai).not.toHaveBeenCalled();
  });
});

describe("rate limit", () => {
  const responses = { model: "gpt-6.1-terra", input: "x" };

  it("counts requests per code, and answers 429 over the rate", async () => {
    const token = await share();
    const { code, id } = await createCode(token);
    rate.mockResolvedValue({ success: false });
    for (const res of [
      await call("/openai/responses", { method: "POST", token: code, ...json(responses) }),
      await call("/openai/audio/transcriptions", { method: "POST", token: code, body: new FormData() }),
      await call("/openai/realtime", { token: code, headers: { Upgrade: "websocket" } }),
    ]) {
      expect(res.status).toBe(429);
    }
    expect(rate).toHaveBeenCalledTimes(3);
    expect(rate).toHaveBeenCalledWith({ key: id });
    expect(openai).not.toHaveBeenCalled();
  });

  it("does not count unknown or refused codes, and does not hide why a code stopped", async () => {
    const token = await share();
    const { code } = await createCode(token, { dailyLimitUsd: 1 });
    await addUsage(db, (await codeByValue(db, code))!.id, 1, Date.now());
    rate.mockResolvedValue({ success: false });
    const res = await call("/openai/responses", { method: "POST", token: code, ...json(responses) });
    expect(res.status).toBe(403);
    expect(res.headers.get("X-Gift-Refusal")).toBe("limit");
    await expect(res.json()).resolves.toEqual({ error: "limit" });
    await call("/openai/responses", { method: "POST", token: "nope", ...json(responses) });
    expect(rate).not.toHaveBeenCalled();
  });

  it("is not asked by /gift, which only reads the state", async () => {
    const { code } = await createCode(await share());
    expect((await call("/gift", { token: code })).status).toBe(200);
    expect(rate).not.toHaveBeenCalled();
  });
});

describe("daily limit", () => {
  const NOW = Date.UTC(2026, 10, 6, 12);
  const DAY = 24 * 60 * 60 * 1000;
  const responses = { model: "gpt-6.1-terra", input: "x" };

  /** 使われた額を D1 に足す */
  async function spend(code: string, usd: number, now: number) {
    await addUsage(db, (await codeByValue(db, code))!.id, usd, now);
  }

  beforeEach(() => {
    vi.spyOn(Date, "now").mockReturnValue(NOW);
    vi.mocked(openai).mockImplementation(async () => Response.json({ usage: {} }));
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("defaults to the same amount for codes that do not choose one", async () => {
    expect(DEFAULT_DAILY_LIMIT_USD).toBe(60);
    const token = await share();
    // 古いアプリは上限を送らない。マイグレーションの前からあるコードも同じ（NULL）
    await createCode(token);
    await env.DB.prepare("UPDATE codes SET daily_limit_usd = NULL").run();
    const [listed] = await listCodes(token);
    // 既定か決めた額かは見分けられ、実際に効く額も分かる
    expect(listed!.dailyLimitUsd).toBeNull();
    expect(listed!.effectiveLimitUsd).toBe(60);
    expect(listed!.dayUsd).toBe(0);
  });

  it("is chosen per code, and can go back to the default", async () => {
    const token = await share();
    const { id } = await createCode(token, { dailyLimitUsd: 5 });
    expect((await listCodes(token))[0]).toMatchObject({ dailyLimitUsd: 5, effectiveLimitUsd: 5 });
    const patch = (dailyLimitUsd: unknown) =>
      call(`/share/codes/${id}`, { method: "PATCH", token, ...json({ dailyLimitUsd }) });
    await patch(12.5);
    expect((await listCodes(token))[0]!.dailyLimitUsd).toBe(12.5);
    await patch(null);
    expect((await listCodes(token))[0]).toMatchObject({ dailyLimitUsd: null, effectiveLimitUsd: 60 });
  });

  it("rejects a limit that is not a positive amount", async () => {
    const token = await share();
    for (const dailyLimitUsd of [0, -1, "20"]) {
      expect((await call("/share/codes", { method: "POST", token, ...json({ dailyLimitUsd }) })).status).toBe(400);
    }
  });

  it("refuses every relay and tells /gift once the limit is reached", async () => {
    const token = await share();
    const { code } = await createCode(token, { dailyLimitUsd: 1 });
    await spend(code, 0.99, NOW);
    expect((await call("/gift", { token: code })).status).toBe(200);
    expect((await call("/openai/responses", { method: "POST", token: code, ...json(responses) })).status).toBe(200);

    await spend(code, 0.01, NOW);
    const gift = await call("/gift", { token: code });
    expect([gift.status, await gift.json()]).toEqual([403, { error: "limit" }]);
    vi.mocked(openai).mockClear();
    const refused = await call("/openai/responses", { method: "POST", token: code, ...json(responses) });
    expect([refused.status, await refused.json()]).toEqual([403, { error: "limit" }]);
    const form = new FormData();
    form.set("model", "gpt-transcribe");
    form.set("file", new File([wav(1)], "segment.wav"));
    const batch = await call("/openai/audio/transcriptions", { method: "POST", token: code, body: form });
    expect(batch.status).toBe(403);
    const realtime = await call("/openai/realtime", { token: code, headers: { Upgrade: "websocket" } });
    expect(realtime.status).toBe(403);
    expect(openai).not.toHaveBeenCalled();
  });

  it("comes back the next UTC day, while the month keeps adding up", async () => {
    const token = await share();
    const { code } = await createCode(token, { dailyLimitUsd: 1 });
    await spend(code, 1, NOW);
    expect((await listCodes(token))[0]!.dayUsd).toBe(1);

    // UTC の 0 時をまたいだら戻る
    vi.mocked(Date.now).mockReturnValue(Date.UTC(2026, 10, 7));
    expect((await call("/gift", { token: code })).status).toBe(200);
    expect((await listCodes(token))[0]!.dayUsd).toBe(0);

    await spend(code, 0.25, Date.now());
    const [listed] = await listCodes(token);
    expect(listed!.dayUsd).toBeCloseTo(0.25);
    expect(listed!.monthUsd).toBeCloseTo(1.25);
    expect(listed!.usd).toBeCloseTo(1.25);
  });

  it("raising the limit lets the code go on the same day", async () => {
    const token = await share();
    const { code, id } = await createCode(token, { dailyLimitUsd: 1 });
    await spend(code, 1, NOW);
    expect((await call("/gift", { token: code })).status).toBe(403);
    await call(`/share/codes/${id}`, { method: "PATCH", token, ...json({ dailyLimitUsd: 2 }) });
    expect((await call("/gift", { token: code })).status).toBe(200);
  });

  it("does not hide a stronger reason", async () => {
    const token = await share();
    const { code, id } = await createCode(token, { dailyLimitUsd: 1, expiresAt: NOW + DAY });
    await spend(code, 1, NOW);
    await call(`/share/codes/${id}`, { method: "PATCH", token, ...json({ revoked: true }) });
    expect(await (await call("/gift", { token: code })).json()).toEqual({ error: "revoked" });
  });

  it("two codes do not share a day's usage", async () => {
    const token = await share();
    const first = await createCode(token, { dailyLimitUsd: 1 });
    const second = await createCode(token, { dailyLimitUsd: 1 });
    await spend(first.code, 1, NOW);
    expect((await call("/gift", { token: first.code })).status).toBe(403);
    expect((await call("/gift", { token: second.code })).status).toBe(200);
  });
});

/** Workers の WebSocket の代わり。送ったものと閉じた理由を覚える */
class FakeSocket extends EventTarget {
  readyState = 1;
  sent: unknown[] = [];
  closed?: { code: number; reason: string };
  accept() {}
  send(data: unknown) {
    this.sent.push(data);
  }
  close(code: number, reason: string) {
    this.closed = { code, reason };
    this.readyState = 3;
  }
  message(data: unknown) {
    this.dispatchEvent(new MessageEvent("message", { data }));
  }
}

/** Workers の WebSocket の仕組みを、`client()`（アプリの側。テストの中で作り直せる）を返す偽物にする */
function stubSockets(client: () => FakeSocket) {
  vi.stubGlobal("WebSocket", { OPEN: 1 });
  vi.stubGlobal("WebSocketPair", function () {
    return { 0: new FakeSocket(), 1: client() };
  });
  // Node の Response は 101 を作れない
  vi.stubGlobal(
    "Response",
    class extends Response {
      constructor(body: BodyInit | null, init: ResponseInit) {
        super(body, init?.status === 101 ? {} : init);
      }
    },
  );
}

describe("realtime relay", () => {

  // crates/core/src/stt.rs の connect / append / close（v0.2.13 から変わっていない）
  const sessionUpdate = (input: Record<string, unknown> = {}, session: Record<string, unknown> = {}) =>
    JSON.stringify({
      type: "session.update",
      session: {
        type: "transcription",
        audio: {
          input: {
            format: { type: "audio/pcm", rate: 24_000 },
            transcription: {
              model: "gpt-live-transcribe",
              languages: ["ja"],
              prompt: "Live commentary",
              keywords: ["ヘイズ"],
              delay: "low",
            },
            turn_detection: null,
            ...input,
          },
        },
        ...session,
      },
    });
  const append = (bytes = 4_800) =>
    JSON.stringify({ type: "input_audio_buffer.append", audio: btoa("\0".repeat(bytes)) });

  let client: FakeSocket;
  let upstream: FakeSocket;
  let code: string;
  const NOW = Date.UTC(2026, 10, 6, 12);

  beforeEach(async () => {
    client = new FakeSocket();
    upstream = new FakeSocket();
    vi.spyOn(Date, "now").mockReturnValue(NOW);
    stubSockets(() => client);
    vi.mocked(openai).mockResolvedValue({ webSocket: upstream } as unknown as Response);
    const token = await share();
    ({ code } = await createCode(token, { dailyLimitUsd: 0.001 }));
    const gift = (await codeByValue(db, code))!;
    await relay(db, gift, KEY, ctx);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function settled() {
    await Promise.allSettled(pending);
    pending = [];
  }

  it("passes everything v0.2.13 sends, and counts the audio", async () => {
    const messages = [
      sessionUpdate(),
      append(),
      JSON.stringify({ type: "input_audio_buffer.commit", event_id: "commit-seg-1" }),
      JSON.stringify({ type: "input_audio_buffer.clear" }),
    ];
    for (const message of messages) client.message(message);
    expect(upstream.sent).toEqual(messages);
    expect(client.closed).toBeUndefined();

    // 切れたときに、送った音声の分（0.1 秒）を書く
    client.dispatchEvent(new CloseEvent("close", { code: 1000 }));
    await settled();
    expect((await codeByValue(db, code))!.dayUsd).toBeCloseTo((0.1 / 60) * 0.017, 8);
  });

  it.each([
    ["a-law", { format: { type: "audio/pcma" } }],
    ["u-law", { format: { type: "audio/pcmu" } }],
    ["another rate", { format: { type: "audio/pcm", rate: 16_000 } }],
    ["no format", { format: undefined }],
  ])("refuses a session.update with %s", (_, input) => {
    client.message(sessionUpdate(input));
    expect(client.closed).toEqual({ code: 1008, reason: "audio format not allowed" });
    expect(upstream.sent).toEqual([]);
  });

  it("refuses a later session.update that changes the format", () => {
    client.message(sessionUpdate());
    client.message(sessionUpdate({ format: { type: "audio/pcmu" } }));
    expect(client.closed?.code).toBe(1008);
    expect(upstream.sent).toHaveLength(1);
  });

  it("refuses a session that is not transcription", () => {
    client.message(sessionUpdate({}, { type: "realtime" }));
    expect(client.closed?.code).toBe(1008);
  });

  it.each([
    ["a binary frame", new ArrayBuffer(8)],
    ["text that is not JSON", "hello"],
    ["JSON null", "null"],
    ["an event the app does not send", JSON.stringify({ type: "response.create" })],
    ["audio that is not text", JSON.stringify({ type: "input_audio_buffer.append", audio: 1 })],
  ])("refuses %s", (_, data) => {
    client.message(sessionUpdate());
    client.message(data);
    expect(client.closed).toEqual({ code: 1008, reason: "message not allowed" });
    expect(upstream.sent).toHaveLength(1);
  });

  it("needs session.update first", () => {
    client.message(append());
    expect(client.closed?.code).toBe(1008);
    expect(upstream.sent).toEqual([]);
  });

  it("cuts the session when the audio reaches the daily limit", async () => {
    client.message(sessionUpdate());
    // 上限の $0.001 を超える音声（約 8 秒）を、30 秒の確認を待たずに一度に送る
    client.message(append(400_000));
    // アプリは reason を見て、張り直さずに知らせる（crates/core/src/stt.rs）
    expect(client.closed).toEqual({ code: 1008, reason: "limit" });
    expect(upstream.closed?.code).toBe(1008);
    expect(upstream.sent).toHaveLength(1);
    await settled();
    expect((await codeByValue(db, code))!.dayUsd).toBeCloseTo((400_000 / 2 / 24_000 / 60) * 0.017, 8);
  });

  it("adds up audio sent faster than real time", () => {
    client.message(sessionUpdate());
    // 1 回は上限に届かない 2 秒ずつ（約 $0.00057）。同じ時刻のうちに 2 回目で超える
    client.message(append(96_000));
    expect(client.closed).toBeUndefined();
    client.message(append(96_000));
    expect(client.closed).toEqual({ code: 1008, reason: "limit" });
  });

  it("starts from what was used today", async () => {
    await env.DB.prepare("UPDATE codes SET day = ?, day_usd = 0.0009").bind(day(NOW)).run();
    client = new FakeSocket();
    upstream = new FakeSocket();
    vi.mocked(openai).mockResolvedValue({ webSocket: upstream } as unknown as Response);
    await relay(db, (await codeByValue(db, code))!, KEY, ctx);
    client.message(sessionUpdate());
    client.message(append(96_000));
    expect(client.closed).toEqual({ code: 1008, reason: "limit" });
  });

  it("takes in what another session used at each check", async () => {
    client.message(sessionUpdate());
    await env.DB.prepare("UPDATE codes SET day = ?, day_usd = 0.0009").bind(day(NOW)).run();
    vi.mocked(Date.now).mockReturnValue(NOW + 30_000);
    client.message(append());
    await settled();
    expect(client.closed).toBeUndefined();
    client.message(append(96_000));
    expect(client.closed).toEqual({ code: 1008, reason: "limit" });
  });

  it("gives the general reason when a code is stopped", async () => {
    client.message(sessionUpdate());
    await env.DB.prepare("UPDATE codes SET revoked = 1").run();
    vi.mocked(Date.now).mockReturnValue(NOW + 30_000);
    client.message(append());
    await settled();
    expect(client.closed).toEqual({ code: 1008, reason: "gift code is no longer usable" });
  });

  it("cuts the session when the OpenAI key is no longer deposited", async () => {
    client.message(sessionUpdate());
    const issuer = (await codeByValue(db, code))!.issuerId;
    await env.DB.prepare("INSERT INTO issuer_keys VALUES (?, 'xai', 'h', 'e')").bind(issuer).run();
    await env.DB.prepare("DELETE FROM issuer_keys WHERE provider = 'openai'").run();
    vi.mocked(Date.now).mockReturnValue(NOW + 30_000);
    client.message(append());
    await settled();
    expect(client.closed).toEqual({ code: 1008, reason: "gift code is no longer usable" });
  });

  it("keeps going under the limit", async () => {
    client.message(sessionUpdate());
    vi.mocked(Date.now).mockReturnValue(NOW + 30_000);
    client.message(append());
    await settled();
    expect(client.closed).toBeUndefined();
  });

  it("cuts the session when the code is gone, though the usage can no longer be written", async () => {
    client.message(sessionUpdate());
    client.message(append());
    // 「おすそ分けをやめる」はコードも消すので、使われた額は FK 違反で書けない
    await env.DB.prepare("DELETE FROM issuers").run();
    vi.mocked(Date.now).mockReturnValue(NOW + 30_000);
    client.message(append());
    await settled();
    expect(client.closed).toEqual({ code: 1008, reason: "gift code is no longer usable" });
  });

  it("keeps the amount when writing it fails, and writes it the next time", async () => {
    const batch = vi
      .fn((statements: D1PreparedStatement[]) => env.DB.batch(statements))
      .mockRejectedValueOnce(new Error("D1 is down"));
    const failing = open({ prepare: (sql: string) => env.DB.prepare(sql), batch } as unknown as D1Database);
    client = new FakeSocket();
    upstream = new FakeSocket();
    vi.mocked(openai).mockResolvedValue({ webSocket: upstream } as unknown as Response);
    await relay(failing, (await codeByValue(db, code))!, KEY, ctx);
    client.message(sessionUpdate());
    client.message(append());
    vi.mocked(Date.now).mockReturnValue(NOW + 30_000);
    client.message(append());
    await settled();
    expect(batch).toHaveBeenCalledOnce();
    expect(client.closed).toBeUndefined();
    client.dispatchEvent(new CloseEvent("close", { code: 1000 }));
    await settled();
    expect((await codeByValue(db, code))!.dayUsd).toBeCloseTo((0.2 / 60) * 0.017, 8);
  });

  it("counts an amount once when the session closes during a write", async () => {
    client.message(sessionUpdate());
    client.message(append());
    vi.mocked(Date.now).mockReturnValue(NOW + 30_000);
    client.message(append());
    client.dispatchEvent(new CloseEvent("close", { code: 1000 }));
    await settled();
    expect((await codeByValue(db, code))!.dayUsd).toBeCloseTo((0.2 / 60) * 0.017, 8);
  });
});

describe("beta", () => {
  /** その所（IP アドレス）からコードをもらう */
  const join = (ip = "203.0.113.1") =>
    call("/beta/codes", { method: "POST", headers: { "CF-Connecting-IP": ip } });
  const joined = async (ip?: string) => ((await (await join(ip)).json()) as { code: string }).code;
  const dayUsd = async (code: string) => (await codeByValue(db, code))!.dayUsd;

  const wavPart = { type: "input_audio", input_audio: { data: "data:audio/wav;base64,AAAA" } };
  const transcription = (parts: unknown[] = [{ type: "input_text", text: "ヘイズ、" }, wavPart]) => ({
    model: "qwen-audio-3.1-asr-flash",
    input: { messages: [{ role: "user", content: parts }] },
    parameters: { format: "wav", sample_rate: "16000", language_hints: ["ja"] },
  });
  const QWEN_PATH = "/api/v1/services/aigc/multimodal-generation/generation";
  const transcribe = (code: string, body: unknown = transcription()) =>
    call(`/qwen${QWEN_PATH}`, { method: "POST", token: code, ...json(body) });

  it("gives a code that relays Qwen and OpenRouter with the developer's keys, under a small daily limit", async () => {
    const code = await joined();
    const gift = await call("/gift", { token: code });
    expect(await gift.json()).toEqual({ expiresAt: null, providers: ["qwen", "openrouter"] });
    expect((await codeByValue(db, code))!.dailyLimitUsd).toBe(0.5);
    // おすそ分けの口（OpenAI）には使えない
    const res = await call("/openai/responses", { method: "POST", token: code, ...json({ model: "gpt-6-luna", input: "x" }) });
    expect([res.status, res.headers.get("X-Gift-Refusal")]).toEqual([403, "unsupported"]);
    expect(openai).not.toHaveBeenCalled();
  });

  it("is closed without the developer's keys", async () => {
    env.BETA_QWEN_KEY = undefined;
    env.BETA_OPENROUTER_KEY = undefined;
    expect((await join()).status).toBe(503);
  });

  it("gives a few codes per place a day", async () => {
    for (let i = 0; i < 3; i++) expect((await join()).status).toBe(200);
    const refused = await join();
    expect([refused.status, await refused.json()]).toEqual([429, { error: "tooMany" }]);
    // ほかの所には出す。IP アドレスそのものは保存しない
    expect((await join("203.0.113.2")).status).toBe(200);
    const { results } = await env.DB.prepare("SELECT place FROM beta_grants").all<{ place: string }>();
    expect(results.every((r) => !r.place.includes("203.0.113"))).toBe(true);
  });

  it("transcribes through Qwen with only the app's request, and counts the tokens", async () => {
    const code = await joined();
    vi.mocked(qwen).mockResolvedValue(
      Response.json({ output: { text: "ヘイズ強い" }, usage: { input_tokens: 200_000, output_tokens: 100_000 } }),
    );
    const res = await transcribe(code, { ...transcription(), extra: true });
    expect(await res.json()).toMatchObject({ output: { text: "ヘイズ強い" } });
    const [path, key, body] = vi.mocked(qwen).mock.calls[0]!;
    expect([path, key, body]).toEqual([QWEN_PATH, "sk-beta-qwen", transcription()]);
    // 200,000 × $0.15 + 100,000 × $0.47（100 万トークンあたり）
    expect(await dayUsd(code)).toBeCloseTo(0.077);

    // ほかのモデル、音声がない・2 つある、WAV でない、長すぎる用語の文
    const mp3 = { type: "input_audio", input_audio: { data: "data:audio/mpeg;base64,AAAA" } };
    for (const bad of [
      { ...transcription(), model: "qwen3-max" },
      transcription([{ type: "input_text", text: "ヘイズ" }]),
      transcription([wavPart, wavPart]),
      transcription([mp3]),
      transcription([{ type: "input_text", text: "x".repeat(501) }, wavPart]),
      { ...transcription(), parameters: { enable_search: true } },
    ]) {
      expect((await transcribe(code, bad)).status).toBe(400);
    }
    expect(qwen).toHaveBeenCalledOnce();
  });

  it("judges terms and translates through OpenRouter with only the cheapest models, and counts the reported cost", async () => {
    const code = await joined();
    const ask = (path: string, body: unknown) => call(`/openrouter${path}`, { method: "POST", token: code, ...json(body) });
    const question = { type: "choice", instructions: "What does ヘイズ mean?", criteria: { "hero:13": "Haze", none: "None" } };
    const judge = { model: "perplexity/pplx-decider-v1.2", state: "ヘイズ強い", questions: { c0: question } };
    vi.mocked(openrouter).mockResolvedValue(
      Response.json({ answers: { c0: { choice: "hero:13" } }, usage: { input_tokens: 100, cost: 0.002 } }),
    );
    expect(await (await ask("/systemone", judge)).json()).toMatchObject({ answers: { c0: { choice: "hero:13" } } });
    expect(vi.mocked(openrouter).mock.calls[0]).toEqual(["/systemone", "sk-beta-openrouter", judge]);
    for (const bad of [
      { ...judge, model: "nace-ai/drex-max" },
      { ...judge, state: ["x"] },
      { ...judge, questions: {} },
      { ...judge, questions: { c0: { ...question, criteria: { a: { image: "x" } } } } },
    ]) {
      expect((await ask("/systemone", bad)).status).toBe(400);
    }

    const translate = {
      model: "tencent/hy-mt2-30b-a3b",
      messages: [{ role: "user", content: "[Source Text]\n⟦Haze⟧強い" }],
      temperature: 0.7,
      top_p: 1,
      max_tokens: 9_999,
    };
    vi.mocked(openrouter).mockResolvedValue(
      Response.json({ choices: [{ message: { content: "⟦Haze⟧ is strong" } }], usage: { prompt_tokens: 1_000_000, completion_tokens: 0 } }),
    );
    expect((await ask("/chat/completions", { ...translate, tools: [{}] })).status).toBe(200);
    // 出力の上限は越えさせず、アプリが送らない項目は通さない
    expect(vi.mocked(openrouter).mock.calls[1]).toEqual([
      "/chat/completions",
      "sk-beta-openrouter",
      { ...translate, max_tokens: 512 },
    ]);
    for (const bad of [
      { ...translate, model: "openai/gpt-6.1-terra" },
      { ...translate, messages: [{ role: "system", content: "x" }, translate.messages[0]] },
      { ...translate, messages: [{ role: "user", content: [{ type: "image_url" }] }] },
    ]) {
      expect((await ask("/chat/completions", bad)).status).toBe(400);
    }
    expect(openrouter).toHaveBeenCalledTimes(2);
    // 判定は答えに入っていた料金。翻訳は料金がなかったので、先に足した見積もり（本文の長さと出力の上限から）のまま
    const sent = JSON.stringify({ ...translate, tools: [{}] });
    expect(await dayUsd(code)).toBeCloseTo(0.002 + (sent.length * 0.074 + 512 * 0.295) / 1e6, 8);
  });

  it("does not relay anything else to Qwen or OpenRouter", async () => {
    const code = await joined();
    for (const path of ["/qwen/chat/completions", "/openrouter/audio/transcriptions"]) {
      const res = await call(path, { method: "POST", token: code });
      expect([res.status, res.headers.get("X-Gift-Refusal")]).toEqual([403, "unsupported"]);
    }
  });

  describe("live captions", () => {
    // crates/core/src/stt/qwen.rs の connect / 音声 / finish
    const header = (action: string) => ({ action, task_id: "0a1b2c", streaming: "duplex" });
    const run = (payload: Record<string, unknown> = {}) =>
      JSON.stringify({
        header: header("run-task"),
        payload: {
          task_group: "audio",
          task: "asr",
          function: "recognition",
          model: "qwen-audio-3.1-asr-flash-streaming",
          parameters: { format: "pcm", sample_rate: 16_000, language_hints: ["ja"] },
          input: {},
          ...payload,
        },
      });
    const audio = (seconds: number) => new ArrayBuffer(seconds * 32_000);
    const result = (input: number, output: number) =>
      JSON.stringify({ header: { event: "result-generated" }, payload: { usage: { input_tokens: input, output_tokens: output } } });
    /** 音声 1 秒の見積もりと、先に足す 30 秒分 */
    const SECOND = (2 * (11 * 0.93 + 3 * 0.7)) / 1e6;
    let client: FakeSocket;
    let upstream: FakeSocket;
    // アプリは、この口だけ認証の方式を小文字で送る
    const connect = (code: string) =>
      call("/qwen/api-ws/v1/inference", { headers: { Upgrade: "websocket", Authorization: `bearer ${code}` } });
    async function settled() {
      await Promise.allSettled(pending);
      pending = [];
    }

    beforeEach(() => {
      client = new FakeSocket();
      upstream = new FakeSocket();
      stubSockets(() => client);
      vi.mocked(qwenLive).mockResolvedValue({ webSocket: upstream } as unknown as Response);
    });
    afterEach(() => vi.unstubAllGlobals());

    it("relays only what the app sends, and counts what Qwen reports", async () => {
      const code = await joined();
      await connect(code);
      expect(vi.mocked(qwenLive).mock.calls).toEqual([["sk-beta-qwen"]]);
      // 先に 30 秒分を足してある
      expect(await dayUsd(code)).toBeCloseTo(30 * SECOND, 10);
      client.message(run({ parameters: { format: "pcm", sample_rate: 16_000, language_hints: ["ja"], vocabulary_id: "x" } }));
      client.message(audio(1));
      client.message(JSON.stringify({ header: header("finish-task"), payload: { input: {} } }));
      expect(upstream.sent).toEqual([
        run(),
        audio(1),
        JSON.stringify({ header: header("finish-task"), payload: { input: {} } }),
      ]);
      upstream.message(result(11, 3));
      expect(client.sent).toEqual([result(11, 3)]);
      client.dispatchEvent(new CloseEvent("close", { code: 1000 }));
      await settled();
      expect(upstream.closed?.code).toBe(1000);
      expect(await dayUsd(code)).toBeCloseTo((11 * 0.93 + 3 * 0.7) / 1e6, 10);
    });

    it.each([
      ["audio before run-task", () => client.message(audio(1)), "run-task comes first"],
      ["another model", () => client.message(run({ model: "qwen-audio-max" })), "model not allowed"],
      ["another message", () => client.message(JSON.stringify({ header: { action: "continue-task" } })), "message not allowed"],
    ])("closes on %s, and counts nothing", async (_, send, reason) => {
      const code = await joined();
      await connect(code);
      send();
      await settled();
      expect(client.closed).toEqual({ code: 1008, reason });
      expect(upstream.sent).toEqual([]);
      expect(await dayUsd(code)).toBe(0);
    });

    it("counts by the length of the audio until Qwen reports, and stops at the daily limit", async () => {
      const code = await joined();
      const { id } = (await codeByValue(db, code))!;
      await env.DB.prepare("UPDATE codes SET daily_limit_usd = ? WHERE id = ?").bind(20 * SECOND, id).run();
      await connect(code);
      client.message(run());
      client.message(audio(30));
      await settled();
      // 30 秒分を実際の額に直したら上限を越えたので、次の分は足さずに切る
      expect(client.closed).toEqual({ code: 1008, reason: "limit" });
      expect(await dayUsd(code)).toBeCloseTo(30 * SECOND, 10);
      // 張り直しても、つなぐ前に断る
      const again = await connect(code);
      expect([again.status, again.headers.get("X-Gift-Refusal")]).toEqual([403, "limit"]);
      expect(qwenLive).toHaveBeenCalledOnce();
    });

    it("stops a code that was revoked while it was connected", async () => {
      const code = await joined();
      const { id } = (await codeByValue(db, code))!;
      await connect(code);
      client.message(run());
      await env.DB.prepare("UPDATE codes SET revoked = 1 WHERE id = ?").bind(id).run();
      // 次の分を足すときに、止められたことが分かる
      client.message(audio(30));
      await settled();
      expect(client.closed).toEqual({ code: 1008, reason: "gift code is no longer usable" });
      const again = await connect(code);
      expect([again.status, again.headers.get("X-Gift-Refusal")]).toEqual([403, "revoked"]);
    });

    it("hides the developer's key trouble, and gives the estimate back", async () => {
      const code = await joined();
      await connect(code);
      client.message(run());
      upstream.message(JSON.stringify({ header: { event: "task-failed", error_code: "InvalidApiKey", error_message: "secret" } }));
      await settled();
      expect(client.sent).toEqual([]);
      expect(client.closed).toEqual({ code: 1011, reason: "upstream" });
      expect(await dayUsd(code)).toBe(0);

      vi.mocked(qwenLive).mockResolvedValue(new Response("no", { status: 401 }));
      const refused = await connect(code);
      expect([refused.status, await refused.json()]).toEqual([502, { error: "upstream" }]);
      expect(await dayUsd(code)).toBe(0);
    });

    it("is not for a shared code, and needs a WebSocket", async () => {
      const { code } = await createCode(await share());
      const shared = await connect(code);
      expect([shared.status, shared.headers.get("X-Gift-Refusal")]).toEqual([403, "unsupported"]);
      const plain = await call("/qwen/api-ws/v1/inference", { token: await joined() });
      expect(plain.status).toBe(426);
      expect(qwenLive).not.toHaveBeenCalled();
    });
  });

  it("does not relay Qwen or OpenRouter for a shared code", async () => {
    const { code } = await createCode(await share());
    const res = await transcribe(code);
    expect([res.status, res.headers.get("X-Gift-Refusal")]).toEqual([403, "unsupported"]);
    expect(qwen).not.toHaveBeenCalled();
  });

  it("stops a code at its daily limit, and every code at the limit of the whole beta", async () => {
    const [mine, other] = [await joined(), await joined("203.0.113.2")];
    const id = async (code: string) => (await codeByValue(db, code))!.id;
    vi.mocked(qwen).mockImplementation(async () => Response.json({ output: { text: "" }, usage: {} }));
    await addUsage(db, await id(mine), 0.5, Date.now());
    const limited = await transcribe(mine);
    expect([limited.status, limited.headers.get("X-Gift-Refusal")]).toEqual([403, "limit"]);
    expect((await transcribe(other)).status).toBe(200);

    // ベータ全体の今日の分が上限に届いたら、どのコードも止める。届いたコードに足した見積もりは戻す
    await env.DB.prepare("INSERT OR REPLACE INTO beta_spend (day, usd) VALUES (?, 5)").bind(day(Date.now())).run();
    const before = await dayUsd(other);
    const full = await transcribe(other);
    expect([full.status, full.headers.get("X-Gift-Refusal")]).toEqual([403, "limit"]);
    expect(await dayUsd(other)).toBeCloseTo(before, 10);
    expect(qwen).toHaveBeenCalledOnce();
  });

  it("holds the limits when requests arrive at the same time", async () => {
    // 同じ所から同時に頼んでも、出すのは 3 つまで
    const answers = await Promise.all(Array.from({ length: 20 }, () => join()));
    expect(answers.filter((res) => res.status === 200)).toHaveLength(3);

    // 上限の手前のコードに同時に送っても、通るのは上限に届くまで（1 回の見積もりは約 $0.0003）
    const code = await joined("203.0.113.9");
    await addUsage(db, (await codeByValue(db, code))!.id, 0.4995, Date.now());
    vi.mocked(qwen).mockImplementation(async () => Response.json({ output: { text: "" }, usage: {} }));
    const sent = await Promise.all(Array.from({ length: 20 }, () => transcribe(code)));
    const passed = sent.filter((res) => res.status === 200).length;
    expect(passed).toBeGreaterThan(0);
    expect(passed).toBeLessThan(5);
  });

  it("hides the developer's upstream trouble, and gives back what it set aside", async () => {
    const code = await joined();
    // 開発者のキーが使えない。そのまま 401 を返すと、アプリは「コードが使えない」と出してしまう
    vi.mocked(qwen).mockResolvedValue(Response.json({ code: "InvalidApiKey", message: "secret detail" }, { status: 401 }));
    const res = await transcribe(code);
    expect([res.status, await res.json()]).toEqual([502, { error: "upstream" }]);
    expect(await dayUsd(code)).toBeCloseTo(0, 10);
    // つながらないときも同じ。回数の上限（429）は、アプリが待って送り直せるように伝える
    vi.mocked(qwen).mockRejectedValue(new Error("network"));
    expect((await transcribe(code)).status).toBe(502);
    vi.mocked(qwen).mockResolvedValue(new Response("slow down", { status: 429 }));
    expect((await transcribe(code)).status).toBe(429);
  });

  it("counts every address of one IPv6 network as the same place", () => {
    expect(network("203.0.113.7")).toBe("203.0.113.7");
    expect(network("2001:db8:0:1:aaaa:bbbb:cccc:dddd")).toBe("2001:db8:0:1");
    expect(network("2001:0DB8::1")).toBe("2001:db8:0:0");
    expect(network("2001:db8:0:1::")).toBe(network("2001:db8:0:1:ffff::1"));
    expect(network("")).toBe("");
  });
});

describe("admin", () => {
  /** 管理画面の Worker の設定。キーの暗号化は、API の Worker の口（ここでは同じ鍵で暗号化する関数）に頼む */
  let adminEnv: AdminEnv;
  beforeEach(() => {
    adminEnv = {
      DB: env.DB,
      TEAM_DOMAIN: "https://team.cloudflareaccess.com",
      POLICY_AUD: "aud",
      VAULT: {
        seal: (plain) => encrypt(plain, env.SHARE_KEY),
        fallbacks: async () => ({ qwen: !!env.BETA_QWEN_KEY, openrouter: !!env.BETA_OPENROUTER_KEY }),
      },
    };
  });
  /** 管理の口を呼ぶ。書き換える要求には、ページの JavaScript が付けるヘッダーを付ける */
  async function manage(path: string, method = "GET", body?: unknown, headers: Record<string, string> = { "X-Admin": "1" }) {
    const init = { method, body: body === undefined ? undefined : JSON.stringify(body), headers: method === "GET" ? {} : headers };
    const res = await admin.request(path, init, adminEnv, ctx as never);
    await Promise.all(pending);
    pending = [];
    return res;
  }
  const join = (ip = "203.0.113.1") => call("/beta/codes", { method: "POST", headers: { "CF-Connecting-IP": ip } });
  const joined = async (ip?: string) => ((await (await join(ip)).json()) as { code: string }).code;
  const translation = {
    model: "tencent/hy-mt2-30b-a3b",
    messages: [{ role: "user", content: "強い" }],
  };
  const translate = (code: string) => call("/openrouter/chat/completions", { method: "POST", token: code, ...json(translation) });
  const answered = () => Response.json({ choices: [{ message: { content: "strong" } }], usage: { cost: 0.01 } });

  it("lets in only who passed Access, and takes changes only from its own page", async () => {
    expect((await manage("/")).status).toBe(200);
    vi.mocked(accessEmail).mockResolvedValue(undefined);
    expect((await manage("/api/summary")).status).toBe(401);
    vi.mocked(accessEmail).mockResolvedValue("dev@example.com");
    // ほかのサイトからの要求（Access のクッキーは付くが、ページのヘッダーは付けられない）
    expect((await manage("/api/settings", "PUT", { closed: true }, {})).status).toBe(403);
    adminEnv.POLICY_AUD = undefined;
    expect((await manage("/api/summary")).status).toBe(503);
  });

  it("shows what the beta spent and stops a code", async () => {
    env.BETA_QWEN_KEY = undefined;
    const code = await joined();
    vi.mocked(openrouter).mockImplementation(async () => answered());
    expect((await translate(code)).status).toBe(200);
    const summary = (await (await manage("/api/summary")).json()) as Record<string, any>;
    expect(summary).toMatchObject({
      email: "dev@example.com",
      closed: false,
      today: { codes: 1, limitUsd: 5 },
      codes: { total: 1, activeToday: 1 },
    });
    expect(summary.today.usd).toBeCloseTo(0.01);
    expect(summary.month.usd).toBeCloseTo(0.01);
    const { codes: listed } = (await (await manage("/api/codes")).json()) as { codes: { id: string; todayUsd: number }[] };
    expect(listed[0]!.todayUsd).toBeCloseTo(0.01);
    expect((await manage(`/api/codes/${listed[0]!.id}`, "PATCH", { revoked: true })).status).toBe(204);
    const stopped = await translate(code);
    expect([stopped.status, stopped.headers.get("X-Gift-Refusal")]).toEqual([403, "revoked"]);
  });

  it("keeps several keys, never shows them, and moves on to the next when one is refused", async () => {
    env.BETA_OPENROUTER_KEY = undefined;
    const add = (key: string, memo = "") => manage("/api/keys", "POST", { provider: "openrouter", key, memo });
    expect((await add("sk-or-0000000000000000-first", "main")).status).toBe(200);
    expect((await add("sk-or-0000000000000000-second")).status).toBe(200);
    // 空や短すぎるキー（打ち間違い。終わりの 4 文字で中身が分かってしまう）は受けない
    expect((await add("")).status).toBe(400);
    expect((await add("sk-or-short")).status).toBe(400);
    expect((await manage("/api/keys", "POST", { provider: "openai", key: "sk-x" })).status).toBe(400);
    const list = async () => ((await (await manage("/api/keys")).json()) as { keys: Record<string, any>[] }).keys;
    const keys = await list();
    expect(keys.map((k) => [k.tail, k.memo, k.enabled])).toEqual([
      ["irst", "main", true],
      ["cond", "", true],
    ]);
    expect(JSON.stringify(keys)).not.toContain("sk-or-");

    // 1 つ目に当たったキーが回数の上限で断られたら、もう 1 つで通す。断られたキーは覚える
    const code = await joined();
    vi.mocked(openrouter).mockImplementation(async (_path, key) =>
      key === "sk-or-0000000000000000-first" ? new Response("slow down", { status: 429 }) : answered(),
    );
    // キーは毎回並べ替えるので、断られるキーに 1 度も当たらないことがないよう、十分な回数を送る
    for (let i = 0; i < 40; i++) expect((await translate(code)).status).toBe(200);
    const [first, second] = await list();
    expect(first!.failures).toBeGreaterThan(0);
    expect([first!.failedStatus, second!.failures]).toEqual([429, 0]);
    // 断られたキーは後回しにするので、次からは通るキーを先に使う
    vi.mocked(openrouter).mockClear();
    await translate(code);
    expect(vi.mocked(openrouter).mock.calls.map((call) => call[1])).toEqual(["sk-or-0000000000000000-second"]);

    // 止めたキーは使わない。どれも使えなければ、その会社は中継しない
    expect((await manage(`/api/keys/${second!.id}`, "PATCH", { enabled: false })).status).toBe(204);
    expect((await translate(code)).status).toBe(429);
    expect((await manage(`/api/keys/${first!.id}`, "DELETE")).status).toBe(204);
    const none = await translate(code);
    expect([none.status, none.headers.get("X-Gift-Refusal")]).toEqual([403, "unsupported"]);
  });

  it("skips a key it cannot read, and does not blame the keys for a refused request", async () => {
    env.BETA_OPENROUTER_KEY = undefined;
    const add = (key: string) => manage("/api/keys", "POST", { provider: "openrouter", key });
    await add("sk-or-0000000000000000-broken");
    await add("sk-or-0000000000000000-works");
    // 暗号化の鍵を替えたあとのような、読めなくなったキー
    await env.DB.prepare("UPDATE beta_keys SET encrypted_key = 'AAAA' WHERE tail = 'oken'").run();
    const code = await joined();
    vi.mocked(openrouter).mockImplementation(async () => answered());
    for (let i = 0; i < 6; i++) expect((await translate(code)).status).toBe(200);
    expect(vi.mocked(openrouter).mock.calls.every((call) => call[1] === "sk-or-0000000000000000-works")).toBe(true);
    const keys = ((await (await manage("/api/keys")).json()) as { keys: Record<string, any>[] }).keys;
    expect(keys.find((k) => k.tail === "oken")!.failures).toBeGreaterThan(0);

    // 本文が断られた（403）のは、キーのせいではない。ほかのキーに回さず、キーにも記録しない
    await env.DB.prepare("DELETE FROM beta_keys WHERE tail = 'oken'").run();
    await add("sk-or-0000000000000000-other");
    vi.mocked(openrouter).mockClear();
    vi.mocked(openrouter).mockImplementation(async () => new Response("moderation", { status: 403 }));
    expect((await translate(code)).status).toBe(502);
    expect(openrouter).toHaveBeenCalledOnce();
    const after = ((await (await manage("/api/keys")).json()) as { keys: Record<string, any>[] }).keys;
    expect(after.every((k) => k.failures === 0)).toBe(true);
  });

  it("serves the page so that only its own script runs, and nothing is framed or kept", async () => {
    const res = await manage("/");
    const csp = res.headers.get("Content-Security-Policy")!;
    const html = await res.text();
    const script = html.slice(html.indexOf("<script>") + "<script>".length, html.indexOf("</script>"));
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(script));
    expect(csp).toContain(`script-src 'sha256-${btoa(String.fromCharCode(...new Uint8Array(digest)))}'`);
    expect(csp).toContain("frame-ancestors 'none'");
    expect(res.headers.get("Cache-Control")).toBe("no-store");
    expect((await manage("/api/summary")).headers.get("Cache-Control")).toBe("no-store");
  });

  it("closes the beta: no new codes, no relay, and opens it again", async () => {
    const code = await joined();
    vi.mocked(openrouter).mockImplementation(async () => answered());
    expect((await manage("/api/settings", "PUT", { closed: true })).status).toBe(204);
    expect((await join("203.0.113.5")).status).toBe(503);
    const closed = await translate(code);
    expect([closed.status, closed.headers.get("X-Gift-Refusal")]).toEqual([403, "unsupported"]);
    expect((await manage("/api/settings", "PUT", { closed: false })).status).toBe(204);
    expect((await translate(code)).status).toBe(200);
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
