import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { getPlatformProxy } from "wrangler";
import { accessEmail } from "./access";
import { admin, type AdminEnv } from "./admin";
import { network } from "./beta";
import { codeByValue, day, open, type Db } from "./db";
import { app, type Env } from "./index";
import { encrypt } from "./secrets";
import { openrouter, qwen, qwenLive } from "./upstream";

vi.mock("./upstream", () => ({ openrouter: vi.fn(), qwen: vi.fn(), qwenLive: vi.fn() }));
vi.mock("./access", () => ({ accessEmail: vi.fn() }));

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
    CODE_RATE: { limit: rate },
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
  for (const table of ["code_usage", "codes", "beta_grants", "beta_spend", "beta_keys", "beta_settings"]) {
    await env.DB.prepare(`DELETE FROM ${table}`).run();
  }
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

describe("beta", () => {
  /** その所（IP アドレス）からコードをもらう */
  const join = (ip = "203.0.113.1") =>
    call("/beta/codes", { method: "POST", headers: { "CF-Connecting-IP": ip } });
  const joined = async (ip?: string) => ((await (await join(ip)).json()) as { code: string }).code;
  const dayUsd = async (code: string) => (await codeByValue(db, code))!.dayUsd;
  /** そのコードが今日使った額を足す */
  const spend = async (code: string, usd: number) => {
    const { id } = (await codeByValue(db, code))!;
    await env.DB.prepare("UPDATE codes SET day = ?, day_usd = day_usd + ? WHERE id = ?").bind(day(Date.now()), usd, id).run();
  };

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
    expect(await gift.json()).toEqual({ providers: ["qwen", "openrouter"] });
    expect((await codeByValue(db, code))!.dailyLimitUsd).toBe(0.5);
    // 0.7 までのギフトコードの口は、もうない
    for (const path of ["/openai/responses", "/xai/stt", "/share/codes"]) {
      expect((await call(path, { method: "POST", token: code, ...json({}) })).status).toBe(404);
    }
  });

  it("tells the app why a code cannot be used, without counting the request", async () => {
    expect((await call("/gift", { token: "0000-0000-0000" })).status).toBe(401);
    expect((await call("/gift")).status).toBe(401);
    const code = await joined();
    await spend(code, 0.5);
    const limited = await call("/gift", { token: code });
    expect([limited.status, await limited.json()]).toEqual([403, { error: "limit" }]);
    await env.DB.prepare("UPDATE codes SET revoked = 1").run();
    expect(await (await call("/gift", { token: code })).json()).toEqual({ error: "revoked" });
    expect(rate).not.toHaveBeenCalled();
  });

  it("counts requests per code, and answers 429 over the rate, after the reasons that stop a code", async () => {
    const code = await joined();
    rate.mockResolvedValue({ success: false });
    expect((await transcribe(code)).status).toBe(429);
    expect(rate).toHaveBeenCalledWith({ key: (await codeByValue(db, code))!.id });
    rate.mockClear();
    // 知らないコードと、上限に届いたコードは数えない。止まる理由を 429 で隠さない
    await transcribe("nope");
    await spend(code, 0.5);
    const limited = await transcribe(code);
    expect([limited.status, limited.headers.get("X-Gift-Refusal")]).toEqual([403, "limit"]);
    expect(rate).not.toHaveBeenCalled();
    expect(qwen).not.toHaveBeenCalled();
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
      expect(client.closed).toEqual({ code: 1008, reason: "code is no longer usable" });
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

    it("needs a WebSocket", async () => {
      const plain = await call("/qwen/api-ws/v1/inference", { token: await joined() });
      expect(plain.status).toBe(426);
      expect(qwenLive).not.toHaveBeenCalled();
    });
  });

  it("stops a code at its daily limit, and every code at the limit of the whole beta", async () => {
    const [mine, other] = [await joined(), await joined("203.0.113.2")];
    vi.mocked(qwen).mockImplementation(async () => Response.json({ output: { text: "" }, usage: {} }));
    await spend(mine, 0.5);
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
    await spend(code, 0.4995);
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

  it("sets the daily limits: the whole beta and each code, old codes included", async () => {
    const code = await joined();
    vi.mocked(openrouter).mockImplementation(async () => answered());
    const limits = async () => {
      const summary = (await (await manage("/api/summary")).json()) as Record<string, any>;
      return [summary.today.limitUsd, summary.limits.codeDailyUsd];
    };
    expect(await limits()).toEqual([5, 0.5]);
    // 読めない値は断り、何も変えない
    for (const body of [{ dailyUsd: 0 }, { dailyUsd: -1 }, { dailyUsd: "9" }, { dailyUsd: 1001 }, { dailyUsd: 9, other: 1 }, {}]) {
      expect((await manage("/api/settings", "PUT", body)).status).toBe(400);
    }
    expect(await limits()).toEqual([5, 0.5]);
    expect((await translate(code)).status).toBe(200);
    // 1 つのコードの上限を、使った額より下げる。出してあるコードも断る
    expect((await manage("/api/settings", "PUT", { codeDailyUsd: 0.005 })).status).toBe(204);
    expect(await limits()).toEqual([5, 0.005]);
    const mine = await translate(code);
    expect([mine.status, mine.headers.get("X-Gift-Refusal")]).toEqual([403, "limit"]);
    // 戻すと通る。全体の上限を下げると、また断る
    expect((await manage("/api/settings", "PUT", { codeDailyUsd: 0.5, dailyUsd: 0.005 })).status).toBe(204);
    expect(await limits()).toEqual([0.005, 0.5]);
    const all = await translate(code);
    expect([all.status, all.headers.get("X-Gift-Refusal")]).toEqual([403, "limit"]);
    expect((await manage("/api/settings", "PUT", { dailyUsd: 5 })).status).toBe(204);
    expect((await translate(code)).status).toBe(200);
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
