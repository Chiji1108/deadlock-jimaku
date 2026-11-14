// API（api.jimaku.deadlock.jp。DESIGN.md 17 章）。パスは版（/v1）で区切る。
// ベータ期間中の無料の提供: アプリが自動でもらうコードを見分けて、開発者のキーで Qwen と OpenRouter に中継する。
// 口の名前（/gift、X-Gift-Refusal）は、人のキーで中継するギフトコード（0.7 まで）のころのもの。出ているアプリが使うので変えない
import { Hono, type Context } from "hono";
import { codeByValue, open, refusal, type Code, type Db } from "./db";
import {
  betaClosed,
  betaProviders,
  betaRelayKeys,
  grantBetaCode,
  keyFailed,
  network,
  reserveBeta,
  settleBeta,
  type BetaEnv,
  type BetaProvider,
  type RelayKey,
} from "./beta";
import { OPENROUTER_FALLBACK, QWEN_AUDIO, tokensOf } from "./price";
import { relayQwenLive } from "./qwenLive";
import { sha256 } from "./secrets";
import { openrouter, qwen } from "./upstream";

export interface Env extends BetaEnv {
  DB: D1Database;
  /** 開発者のキーを暗号化する鍵（32 バイトの base64）。`wrangler secret put SHARE_KEY` */
  SHARE_KEY: string;
  /** コードごとの 1 分あたりの要求数の上限（値は wrangler.jsonc。DESIGN.md 17.2） */
  CODE_RATE: RateLimit;
}

type App = { Bindings: Env; Variables: { db: Db } };
type Ctx = Context<App>;

export const app = new Hono<App>().basePath("/v1");

app.use(async (c, next) => {
  c.set("db", open(c.env.DB));
  await next();
});

/** 用語の判定の問いの数の上限。アプリは 1 発話で候補の数だけ問い（多くて 10 ほど）を送る */
const MAX_QUESTIONS = 40;

/** ベータで中継するモデル（これだけ。いちばん安い組み合わせ。DESIGN.md 17.2） */
const BETA_JUDGE = "perplexity/pplx-decider-v1.2";
const BETA_TRANSLATOR = "tencent/hy-mt2-30b-a3b";

/** Qwen の文字起こしの本文の上限（文字数）。発話はいちばん長くて 13 秒ほど（WAV で 0.4 MB、base64 で 0.6 MB） */
const MAX_QWEN_BODY = 2_000_000;
/** 文字起こしに添える文（用語）の上限。アプリは 400 文字まで送る */
const MAX_QWEN_CONTEXT = 500;
/** 用語の判定の本文の上限。アプリは 1 発話で、多くて 10 ほどの問い（1 問 1,000 文字ほど）を送る */
const MAX_JUDGE_BODY = 32_000;
/** 翻訳の本文の上限。アプリは 1 発話と短い指示文（400 文字ほど）を送る */
const MAX_TRANSLATION_BODY = 8_000;
/** 翻訳の出力の上限（アプリが送る値） */
const MAX_TRANSLATION_TOKENS = 512;

/** コードの状態。アプリは動いていない字幕を始めるときに、これで確かめる */
app.get("/gift", async (c) => {
  const code = await bearerCode(c);
  if (!code) return c.json({ error: "unknown" }, 401);
  const error = refusal(code, Date.now());
  if (error) return c.json({ error }, 403);
  return c.json({ providers: await betaProviders(c.var.db, c.env) });
});

/**
 * ベータのコードを 1 つ出す（DESIGN.md 17.2）。アプリが、ベータのモードを選んだときに 1 度だけ頼む。
 * 誰でも頼めるので、同じ所に出す数と、1 日に出す数を限る。開発者がキーを入れていなければ閉じている
 */
app.post("/beta/codes", async (c) => {
  if ((await betaProviders(c.var.db, c.env)).length === 0) return c.json({ error: "closed" }, 503);
  const now = Date.now();
  const ip = network(c.req.header("CF-Connecting-IP") ?? "");
  const place = await sha256(`beta:${new Date(now).toISOString().slice(0, 10)}:${ip}:${c.env.SHARE_KEY}`);
  const granted = await grantBetaCode(c.var.db, place, now);
  return "code" in granted ? c.json(granted) : c.json(granted, 429);
});

// ---- 中継（開発者のキー） ----

/** OpenRouter の応答の料金。答えに入っていなければ undefined（見積もりで数える） */
function openrouterUsd(usage: unknown): number | undefined {
  const cost = (usage as { cost?: unknown } | undefined)?.cost;
  return typeof cost === "number" ? cost : undefined;
}

/**
 * ベータのコードで外に送る。送る前に、かかりそうな額（多めの見積もり）を先に足し（上限に届いていたら送らない）、
 * 応答のあとに実際の額に直す。額が分からない応答は、見積もりのまま数える。
 * 外の不調（開発者のキーが使えない、残高がないなど）は、利用者には関係がないので、中身を返さず 502 にする
 * （そのまま 401 を返すと、アプリは「コードが使えない」と出してしまう）
 */
async function relayed(
  c: Ctx,
  gift: { code: Code; keys: RelayKey[] },
  estimate: number,
  send: (key: string) => Promise<Response>,
  usd: (answer: Record<string, unknown>) => number | undefined,
): Promise<Response> {
  const now = Date.now();
  const codeId = gift.code.id;
  if (!(await reserveBeta(c.var.db, codeId, estimate, now))) {
    return c.json({ error: "limit" }, 403, { "X-Gift-Refusal": "limit" });
  }
  const settle = (actual: number) => c.executionCtx.waitUntil(settleBeta(c.var.db, codeId, estimate, actual, now));
  // キーが断られたら（使えない、残高がない、回数の上限）、次のキーに回す。キーを読めないとき（暗号化の鍵を
  // 替えた、行が壊れた）も同じ。断られたキーは覚えて、次からは後回しにする
  let res: Response | undefined;
  for (const key of gift.keys) {
    const secret = await key.key().catch(() => undefined);
    const tried = secret === undefined ? undefined : await send(secret).catch(() => undefined);
    void res?.body?.cancel();
    res = tried;
    // 読めなかったキーと、キーのせいで断られたとき（その番号）だけ、次に回す
    const refused = tried && KEY_TROUBLE.includes(tried.status) ? tried.status : undefined;
    const trouble = secret === undefined ? KEY_UNREADABLE : refused;
    if (trouble === undefined) break;
    if (key.id) c.executionCtx.waitUntil(keyFailed(c.var.db, key.id, trouble, now));
  }
  if (!res?.ok) {
    settle(0);
    const status = res?.status === 429 ? 429 : 502;
    return c.json({ error: "upstream" }, status);
  }
  const answer = await res.text();
  settle(usd(parsed<Record<string, unknown>>(answer) ?? {}) ?? estimate);
  return new Response(answer, { headers: { "Content-Type": "application/json" } });
}

/** キーのせいで断られたときの番号（ほかのキーなら通るかもしれない）。403 は入れない（本文が断られたときにも返る） */
const KEY_TROUBLE = [401, 402, 429];
/** キーを読めなかったときに覚える番号 */
const KEY_UNREADABLE = 0;

/** 100 万トークンあたりの単価から、額を出す */
const usdOf = (tokens: number, perMillion: number) => (tokens * perMillion) / 1e6;

/** 確定の文字起こし（Qwen-Audio-3.1-ASR-Flash）。アプリが送る形だけを通す: 用語の文と、WAV が 1 つ */
app.post("/qwen/api/v1/services/aigc/multimodal-generation/generation", async (c) => {
  const gift = await relayCode(c, "qwen");
  if (gift instanceof Response) return gift;
  const text = await c.req.text();
  if (text.length > MAX_QWEN_BODY) return c.json({ error: "tooLarge" }, 413);
  const body = parsed<Record<string, unknown>>(text) ?? {};
  const messages = (body.input as { messages?: unknown } | undefined)?.messages;
  const message = Array.isArray(messages) && messages.length === 1 ? (messages[0] as Record<string, unknown>) : {};
  const parts = Array.isArray(message.content) ? (message.content as Record<string, unknown>[]) : [];
  const context = (p: Record<string, unknown>) =>
    p?.type === "input_text" && typeof p.text === "string" && p.text.length <= MAX_QWEN_CONTEXT ? p.text : undefined;
  const audio = (p: Record<string, unknown>) => {
    const data = (p?.input_audio as { data?: unknown } | undefined)?.data;
    const wav = p?.type === "input_audio" && typeof data === "string" && data.startsWith("data:audio/wav;base64,");
    return wav ? (data as string) : undefined;
  };
  const hints = (body.parameters as { language_hints?: unknown } | undefined)?.language_hints;
  const language = Array.isArray(hints) && hints.length === 1 ? hints[0] : undefined;
  const sounds = parts.flatMap((p) => audio(p) ?? []);
  if (
    body.model !== QWEN_AUDIO.model ||
    message.role !== "user" ||
    sounds.length !== 1 ||
    !parts.every((p) => context(p) !== undefined || audio(p) !== undefined) ||
    typeof language !== "string" ||
    language.length > 8
  ) {
    return c.json({ error: "notAllowed" }, 400);
  }
  // アプリが送る項目だけを作り直して送る（ほかの項目やパラメータは通さない）
  const content = parts.map((p) => {
    const said = context(p);
    return said === undefined
      ? { type: "input_audio", input_audio: { data: audio(p) } }
      : { type: "input_text", text: said };
  });
  const forwarded = {
    model: QWEN_AUDIO.model,
    input: { messages: [{ role: "user", content }] },
    parameters: { format: "wav", sample_rate: "16000", language_hints: [language] },
  };
  // 16kHz・16bit の WAV として、音声 1 秒が入力約 11 トークン。出力は長くても数百トークン。倍を見ておく
  const seconds = (sounds[0]!.length * 0.75) / 32_000;
  const estimate = 2 * (usdOf(seconds * 11 + MAX_QWEN_CONTEXT, QWEN_AUDIO.input) + usdOf(300, QWEN_AUDIO.output));
  const path = "/api/v1/services/aigc/multimodal-generation/generation";
  return relayed(c, gift, estimate, (key) => qwen(path, key, forwarded), (answer) => {
    if (answer.usage === undefined) return undefined;
    const { input, output } = tokensOf(answer.usage);
    return usdOf(input, QWEN_AUDIO.input) + usdOf(output, QWEN_AUDIO.output);
  });
});

/** 途中の字幕（Qwen-Audio-3.1-ASR-Flash-Streaming）。WebSocket をつないで中継する（`qwenLive.ts`） */
app.get("/qwen/api-ws/v1/inference", async (c) => {
  if (c.req.header("Upgrade") !== "websocket") return c.json({ error: "websocket" }, 426);
  const gift = await relayCode(c, "qwen");
  if (gift instanceof Response) return gift;
  const relayed = await relayQwenLive(c.var.db, gift.code.id, gift.keys, c.executionCtx);
  if (relayed instanceof Response) return relayed;
  return relayed.error === "limit"
    ? c.json(relayed, 403, { "X-Gift-Refusal": "limit" })
    : c.json(relayed, 502);
});

/** 用語の判定（Perplexity Decider）。アプリが送る形だけを通す: 文字の state と、文字だけの選択の問い */
app.post("/openrouter/systemone", async (c) => {
  const gift = await relayCode(c, "openrouter");
  if (gift instanceof Response) return gift;
  const text = await c.req.text();
  if (text.length > MAX_JUDGE_BODY) return c.json({ error: "tooLarge" }, 413);
  const { model, state, questions } = parsed<Record<string, unknown>>(text) ?? {};
  const asked = questions !== null && typeof questions === "object" ? Object.entries(questions) : [];
  /** 問いを、アプリが送る項目だけにして作り直す。形が違えば undefined */
  const choice = (q: unknown) => {
    const { type, instructions, criteria } = (q ?? {}) as Record<string, unknown>;
    const options = criteria !== null && typeof criteria === "object" ? Object.entries(criteria) : [];
    const valid =
      type === "choice" &&
      typeof instructions === "string" &&
      options.length > 0 &&
      options.every(([, description]) => typeof description === "string");
    return valid ? { type, instructions, criteria: Object.fromEntries(options) } : undefined;
  };
  const choices = asked.map(([name, q]) => [name, choice(q)] as const);
  if (
    model !== BETA_JUDGE ||
    typeof state !== "string" ||
    choices.length === 0 ||
    choices.length > MAX_QUESTIONS ||
    choices.some(([, q]) => q === undefined)
  ) {
    return c.json({ error: "notAllowed" }, 400);
  }
  const forwarded = { model, state, questions: Object.fromEntries(choices) };
  // 文字数より多いトークンにはならない。出力は数えられない
  const estimate = usdOf(text.length, OPENROUTER_FALLBACK.input);
  return relayed(
    c,
    gift,
    estimate,
    (key) => openrouter("/systemone", key, forwarded),
    (answer) => openrouterUsd(answer.usage),
  );
});

/** 翻訳（Hy-MT2）。アプリが送る形だけを通す: user の文字の 1 通 */
app.post("/openrouter/chat/completions", async (c) => {
  const gift = await relayCode(c, "openrouter");
  if (gift instanceof Response) return gift;
  const text = await c.req.text();
  if (text.length > MAX_TRANSLATION_BODY) return c.json({ error: "tooLarge" }, 413);
  const body = parsed<Record<string, unknown>>(text) ?? {};
  const messages = Array.isArray(body.messages) ? (body.messages as Record<string, unknown>[]) : [];
  const [message] = messages;
  const number = (value: unknown) => (typeof value === "number" ? value : undefined);
  if (
    body.model !== BETA_TRANSLATOR ||
    messages.length !== 1 ||
    message?.role !== "user" ||
    typeof message.content !== "string"
  ) {
    return c.json({ error: "notAllowed" }, 400);
  }
  const forwarded = {
    model: BETA_TRANSLATOR,
    messages: [{ role: "user", content: message.content }],
    temperature: number(body.temperature),
    top_p: number(body.top_p),
    max_tokens: Math.min(number(body.max_tokens) ?? MAX_TRANSLATION_TOKENS, MAX_TRANSLATION_TOKENS),
  };
  const { input, output } = OPENROUTER_FALLBACK;
  const estimate = usdOf(text.length, input) + usdOf(MAX_TRANSLATION_TOKENS, output);
  return relayed(
    c,
    gift,
    estimate,
    (key) => openrouter("/chat/completions", key, forwarded),
    (answer) => openrouterUsd(answer.usage),
  );
});

/** Qwen と OpenRouter のほかの口（途中の字幕、ほかのモデル）は中継しない。アプリは「このコードでは使えない」と出す */
app.all("/qwen/*", (c) => c.json({ error: "unsupported" }, 403, { "X-Gift-Refusal": "unsupported" }));
app.all("/openrouter/*", (c) => c.json({ error: "unsupported" }, 403, { "X-Gift-Refusal": "unsupported" }));

// ---- 共通 ----

function bearer(c: Ctx): string | undefined {
  // 認証の方式の名前は、大文字と小文字を区別しない（アプリは Qwen の途中の字幕だけ小文字で送る）
  return c.req.header("Authorization")?.match(/^Bearer (.+)$/i)?.[1];
}

async function bearerCode(c: Ctx): Promise<Code | null> {
  const code = bearer(c);
  return code ? codeByValue(c.var.db, code) : null;
}

/**
 * 中継に使えるコードと、開発者のその会社のキー（使う順）。使えなければ返す応答。理由はヘッダーにも載せる
 * （WebSocket のクライアントは、断られたときの本文を読みきれるとは限らない）。閉じているときと、その会社のキーが
 * ないときは `unsupported`。429 は 403 のあとに見るので、止まる理由を隠さない。
 * 上限（コードごとと全体）は、送る直前に額を先に足して確かめる（`relayed`）
 */
async function relayCode(c: Ctx, provider: BetaProvider): Promise<{ code: Code; keys: RelayKey[] } | Response> {
  const code = await bearerCode(c);
  if (!code) return c.json({ error: "unknown" }, 401);
  const deny = (error: string) => c.json({ error }, 403, { "X-Gift-Refusal": error });
  const error = refusal(code, Date.now());
  if (error) return deny(error);
  const keys = (await betaClosed(c.var.db)) ? [] : await betaRelayKeys(c.var.db, c.env, provider, c.env.SHARE_KEY);
  if (keys.length === 0) return deny("unsupported");
  const { success } = await c.env.CODE_RATE.limit({ key: code.id });
  if (!success) return c.json({ error: "rate" }, 429);
  return { code, keys };
}

function parsed<T>(text: string): T | undefined {
  try {
    const body: unknown = JSON.parse(text);
    return body !== null && typeof body === "object" ? (body as T) : undefined;
  } catch {
    return undefined;
  }
}

export default app;
