import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { getPlatformProxy } from "wrangler";
import share0001 from "../migrations/0001_share.sql?raw";
import limit0002 from "../migrations/0002_daily_limit.sql?raw";
import { addUsage, codeByValue, DEFAULT_DAILY_LIMIT_USD } from "./db";
import { app, type Env } from "./index";
import { openai, validKey } from "./openai";
import { appendSeconds, tokensUsd, wavSeconds } from "./price";
import { relay } from "./realtime";
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
  const sql = (share0001 + limit0002)
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
      dailyLimitUsd: number;
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

describe("daily limit", () => {
  const NOW = Date.UTC(2026, 10, 6, 12);
  const DAY = 24 * 60 * 60 * 1000;
  const responses = { model: "gpt-6.1-terra", input: "x" };

  /** 使われた額を D1 に足す */
  async function spend(code: string, usd: number, now: number) {
    await addUsage(env.DB, (await codeByValue(env.DB, code))!.id, usd, now);
  }

  beforeEach(() => {
    vi.spyOn(Date, "now").mockReturnValue(NOW);
    vi.mocked(openai).mockImplementation(async () => Response.json({ usage: {} }));
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("defaults to the same amount for codes that do not choose one", async () => {
    expect(DEFAULT_DAILY_LIMIT_USD).toBe(40);
    const token = await share();
    // 古いアプリは上限を送らない。マイグレーションの前からあるコードも同じ（NULL）
    await createCode(token);
    await env.DB.prepare("UPDATE codes SET daily_limit_usd = NULL").run();
    const [listed] = await listCodes(token);
    expect(listed!.dailyLimitUsd).toBe(40);
    expect(listed!.dayUsd).toBe(0);
  });

  it("is chosen per code, and can go back to the default", async () => {
    const token = await share();
    const { id } = await createCode(token, { dailyLimitUsd: 5 });
    expect((await listCodes(token))[0]!.dailyLimitUsd).toBe(5);
    const patch = (dailyLimitUsd: unknown) =>
      call(`/share/codes/${id}`, { method: "PATCH", token, ...json({ dailyLimitUsd }) });
    await patch(12.5);
    expect((await listCodes(token))[0]!.dailyLimitUsd).toBe(12.5);
    await patch(null);
    expect((await listCodes(token))[0]!.dailyLimitUsd).toBe(40);
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

describe("realtime relay", () => {
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
    vi.stubGlobal("WebSocket", { OPEN: 1 });
    vi.stubGlobal("WebSocketPair", function () {
      return { 0: new FakeSocket(), 1: client };
    });
    // Node の Response は 101 を作れない
    vi.stubGlobal(
      "Response",
      class extends Response {
        constructor(body: BodyInit | null, init: ResponseInit) {
          super(body, init.status === 101 ? {} : init);
        }
      },
    );
    vi.mocked(openai).mockResolvedValue({ webSocket: upstream } as unknown as Response);
    const token = await share();
    ({ code } = await createCode(token, { dailyLimitUsd: 0.001 }));
    const gift = (await codeByValue(env.DB, code))!;
    await relay(env.DB, gift, KEY, ctx);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function settled() {
    await Promise.all(pending);
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
    expect((await codeByValue(env.DB, code))!.day_usd).toBeCloseTo((0.1 / 60) * 0.017, 8);
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
    // 上限の $0.001 を超える音声（約 4 秒）を送り、次の書き込みの時刻まで進める
    client.message(append(400_000));
    vi.mocked(Date.now).mockReturnValue(NOW + 30_000);
    client.message(append());
    await settled();
    expect(client.closed).toEqual({ code: 1008, reason: "gift code is no longer usable" });
    expect(upstream.closed?.code).toBe(1008);
  });

  it("keeps going under the limit", async () => {
    client.message(sessionUpdate());
    vi.mocked(Date.now).mockReturnValue(NOW + 30_000);
    client.message(append());
    await settled();
    expect(client.closed).toBeUndefined();
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
