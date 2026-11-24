// API（api.jimaku.deadlock.jp。DESIGN.md 17 章）。パスは版で区切る。/v1 は会社の形の要求をそのまま通す中継（このファイル）、
// /v2 は仕事ごとの口（v2.ts。DESIGN.md 17.4）。/v1 は、仕事の口に移し終えて、古い版のアプリがいなくなったら消す。
// ベータ期間中の無料の提供: アプリが自動でもらうコードを見分けて、開発者のキーで Qwen と OpenRouter に中継する。
// 口の名前（/gift、X-Gift-Refusal）は、人のキーで中継するギフトコード（0.7 まで）のころのもの。出ているアプリが使うので変えない
import { Hono } from "hono";
import { open, refusal, type Code } from "./db";
import {
  betaClosed,
  betaProviders,
  betaRelayKeys,
  grantBetaCode,
  network,
  reserveBeta,
  settleBeta,
  type BetaProvider,
  type RelayKey,
  type Work,
} from "./beta";
import { OPENROUTER_FALLBACK, QWEN_AUDIO, tokensOf } from "./price";
import { relayQwenLive } from "./qwenLive";
import { bearerCode, openrouterUsd, parsed, sent, usdOf, type App, type Ctx, type Env } from "./relay";
import { sha256 } from "./secrets";
import { openrouter, qwen } from "./upstream";
import { status, v2 } from "./v2";

export type { Env };

export const app = new Hono<App>();

app.use(async (c, next) => {
  c.set("db", open(c.env.DB));
  await next();
});

const v1 = new Hono<App>();

/** 用語の判定の問いの数の上限。アプリは 1 発話で候補の数だけ問い（多くて 10 ほど）を送る */
const MAX_QUESTIONS = 40;

/** ベータで中継するモデル（これだけ。いちばん安い組み合わせ。DESIGN.md 17.2）。費用の内訳には、tasks.ts での同じモデルの名前で数える */
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

/** 口の名前（/gift）は、人のキーで中継するギフトコード（0.7 まで）のころのもの。0.9.9 までのアプリが使う */
v1.get("/gift", status);

/**
 * ベータのコードを 1 つ出す（DESIGN.md 17.2）。アプリが、ベータのモードを選んだときに 1 度だけ頼む。
 * 誰でも頼めるので、同じ所に出す数と、1 日に出す数を限る。開発者がキーを入れていなければ閉じている
 */
v1.post("/beta/codes", async (c) => {
  if ((await betaProviders(c.var.db, c.env)).length === 0) return c.json({ error: "closed" }, 503);
  const now = Date.now();
  const ip = network(c.req.header("CF-Connecting-IP") ?? "");
  const place = await sha256(`beta:${new Date(now).toISOString().slice(0, 10)}:${ip}:${c.env.SHARE_KEY}`);
  const granted = await grantBetaCode(c.var.db, place, now);
  return "code" in granted ? c.json(granted) : c.json(granted, 429);
});

// ---- 中継（開発者のキー） ----

/**
 * ベータのコードで外に送る。送る前に、かかりそうな額（多めの見積もり）を先に足し（上限に届いていたら送らない）、
 * 応答のあとに実際の額に直す。額が分からない応答は、見積もりのまま数える。
 * 外の不調（開発者のキーが使えない、残高がないなど）は、利用者には関係がないので、中身を返さず 502 にする
 * （そのまま 401 を返すと、アプリは「コードが使えない」と出してしまう）
 */
async function relayed(
  c: Ctx,
  gift: { code: Code; keys: RelayKey[] },
  work: Work,
  estimate: number,
  send: (key: string) => Promise<Response>,
  usd: (answer: Record<string, unknown>) => number | undefined,
): Promise<Response> {
  const now = Date.now();
  const codeId = gift.code.id;
  if (!(await reserveBeta(c.var.db, codeId, estimate, now))) {
    return c.json({ error: "limit" }, 403, { "X-Gift-Refusal": "limit" });
  }
  const settle = (actual: number) => c.executionCtx.waitUntil(settleBeta(c.var.db, codeId, work, estimate, actual, now));
  const res = await sent(c, gift.keys, send);
  if (!res?.ok) {
    settle(0);
    const status = res?.status === 429 ? 429 : 502;
    return c.json({ error: "upstream" }, status);
  }
  const answer = await res.text();
  settle(usd(parsed<Record<string, unknown>>(answer) ?? {}) ?? estimate);
  return new Response(answer, { headers: { "Content-Type": "application/json" } });
}

/** 確定の文字起こし（Qwen-Audio-3.1-ASR-Flash）。アプリが送る形だけを通す: 用語の文と、WAV が 1 つ */
v1.post("/qwen/api/v1/services/aigc/multimodal-generation/generation", async (c) => {
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
  return relayed(c, gift, { task: "transcribe", model: "qwen-audio" }, estimate, (key) => qwen(path, key, forwarded), (answer) => {
    if (answer.usage === undefined) return undefined;
    const { input, output } = tokensOf(answer.usage);
    return usdOf(input, QWEN_AUDIO.input) + usdOf(output, QWEN_AUDIO.output);
  });
});

/** 途中の字幕（Qwen-Audio-3.1-ASR-Flash-Streaming）。WebSocket をつないで中継する（`qwenLive.ts`） */
v1.get("/qwen/api-ws/v1/inference", async (c) => {
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
v1.post("/openrouter/systemone", async (c) => {
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
    { task: "decide", model: "pplx-decider" },
    estimate,
    (key) => openrouter("/systemone", key, forwarded),
    (answer) => openrouterUsd(answer.usage),
  );
});

/** 翻訳（Hy-MT2）。アプリが送る形だけを通す: user の文字の 1 通 */
v1.post("/openrouter/chat/completions", async (c) => {
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
    { task: "translate", model: "hy-mt2" },
    estimate,
    (key) => openrouter("/chat/completions", key, forwarded),
    (answer) => openrouterUsd(answer.usage),
  );
});

/** Qwen と OpenRouter のほかの口（途中の字幕、ほかのモデル）は中継しない。アプリは「このコードでは使えない」と出す */
v1.all("/qwen/*", (c) => c.json({ error: "unsupported" }, 403, { "X-Gift-Refusal": "unsupported" }));
v1.all("/openrouter/*", (c) => c.json({ error: "unsupported" }, 403, { "X-Gift-Refusal": "unsupported" }));

// ---- 共通 ----

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

app.route("/v1", v1);
app.route("/v2", v2);

export default app;
