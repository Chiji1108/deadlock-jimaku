// 仕事ごとの口（/v2。DESIGN.md 17.4）。アプリは「何をしてほしいか」を送り、モデルと頼み方は Worker が決める（tasks.ts）。
// 断り方は、どの口も同じ（`refuse`）。使う人は、ベータのコード（この PC の印）で見分ける

import { Hono } from "hono";
import { betaClosed, betaProviders, betaRelayKeys, reserveBeta, settleBeta } from "./beta";
import { appVersion, CODE_DAILY_USD, refusal, seen, todayUsd, type Code } from "./db";
import { language } from "./lang";
import { bearerCode, openrouterUsd, parsed, sent, usdOf, type App, type Ctx } from "./relay";
import { MAX_TRANSLATION_TOKENS, openTasks, taskOf, type Model } from "./tasks";
import { chat, wait } from "./upstream";

export const v2 = new Hono<App>();

/**
 * 断る理由。アプリは、理由ごとに続け方を決める:
 * `unknown`（ないコード）、`revoked`（止められた）、`limit`（今日の上限）、`outdated`（この版のアプリには、もう答えない）は、
 * どの仕事も同じに断られるので、サーバーを使わない作り方に替える。`stopped`（この仕事を止めている）は、その工程だけ。
 * `busy`（混んでいる）は、その 1 回だけ
 */
type Refusal = "unknown" | "revoked" | "limit" | "outdated" | "stopped" | "busy";

const STATUS = { unknown: 401, revoked: 403, limit: 403, outdated: 403, stopped: 403, busy: 429 } as const;

const refuse = (c: Ctx, error: Refusal) => c.json({ error }, STATUS[error]);

/**
 * コードの状態。アプリは動いていない字幕を始めるときに、これで確かめる。答えるのは、今頼める仕事、
 * 中継する会社（/v1。仕事の口に移し終えたらなくなる）、今日の残り（0〜1）
 */
export async function status(c: Ctx) {
  const code = await bearerCode(c);
  if (!code) return refuse(c, "unknown");
  const now = Date.now();
  // 覚えられなくても、コードの答えは返す（アプリは、答えが来ないと字幕を始めない）
  const noted = seen(c.var.db, code.id, appVersion(c.req.header("User-Agent")), now);
  c.executionCtx.waitUntil(noted.catch((e: unknown) => console.error("seen", e)));
  const error = refusal(code, now);
  if (error) return refuse(c, error);
  const providers = await betaProviders(c.var.db, c.env);
  const remaining = Math.max(1 - todayUsd(code, now) / (code.dailyLimitUsd ?? CODE_DAILY_USD), 0);
  return c.json({ tasks: await openTasks(c.var.db, providers), providers, remaining });
}

v2.get("/status", status);

/** 仕事を頼めるコード。頼めなければ、断る答え。上限は、送る直前に額を先に足して確かめる（`reserveBeta`） */
async function asker(c: Ctx): Promise<Code | Response> {
  const code = await bearerCode(c);
  if (!code) return refuse(c, "unknown");
  const error = refusal(code, Date.now());
  if (error) return refuse(c, error);
  const { success } = await c.env.CODE_RATE.limit({ key: code.id });
  return success ? code : refuse(c, "busy");
}

/** 1 回に訳す文の長さの上限（文字数）。1 発話は長くても 13 秒ほどで、200 文字に届かない */
const MAX_TEXT = 1000;
/** 翻訳先の数の上限（アプリは 3 つまで選べる） */
const MAX_TARGETS = 4;
/** 訳し方の指示文の長さ（文字数。見積もりに足す） */
const PROMPT_CHARS = 500;
/** 回数の上限（429）で断られたら、待って送り直す（訳が遅れても、抜けるよりよい） */
const RETRY_AFTER_MS = [700, 1500];

/**
 * 翻訳。用語を `⟦英語名⟧` で囲んだ文を、翻訳先ごとに訳す。翻訳先ごとに 1 回ずつ、同時に外に聞く。
 * 訳せなかった翻訳先は、答えに入れない（アプリは、届いた分だけを出す）
 */
v2.post("/translate", async (c) => {
  const code = await asker(c);
  if (code instanceof Response) return code;
  const body = parsed<Record<string, unknown>>(await c.req.text()) ?? {};
  const { text, source } = body;
  const targets = Array.isArray(body.targets) ? [...new Set(body.targets as unknown[])] : [];
  if (
    typeof text !== "string" ||
    text.length === 0 ||
    text.length > MAX_TEXT ||
    typeof source !== "string" ||
    !language(source) ||
    targets.length === 0 ||
    targets.length > MAX_TARGETS ||
    !targets.every((target): target is string => !!language(target))
  ) {
    return c.json({ error: "invalid" }, 400);
  }
  const db = c.var.db;
  const task = await taskOf(db, "translate");
  const { model } = task;
  const closed = !task.enabled || (await betaClosed(db));
  const keys = closed ? [] : await betaRelayKeys(db, c.env, model.provider, c.env.SHARE_KEY);
  if (keys.length === 0) return refuse(c, "stopped");

  // 文字数より多いトークンにはならない。出力は上限まで見ておく
  const one = usdOf(text.length + PROMPT_CHARS, model.price.input) + usdOf(MAX_TRANSLATION_TOKENS, model.price.output);
  const estimate = one * targets.length;
  const now = Date.now();
  if (!(await reserveBeta(db, code.id, estimate, now))) return refuse(c, "limit");

  const asked = await Promise.all(
    targets.map(async (target) => {
      const send = (key: string) => chat(model.provider, key, model.body({ text, source, target }));
      let res = await sent(c, keys, send);
      for (const ms of RETRY_AFTER_MS) {
        if (res?.status !== 429) break;
        await wait(ms);
        res = await sent(c, keys, send);
      }
      const answer = res?.ok ? await res.json<Answer>().catch(() => undefined) : undefined;
      const translated = answer?.choices?.[0]?.message?.content;
      return {
        target,
        translated: typeof translated === "string" ? translated.trim() : "",
        // 答えが来なかった分は数えない。額の分からない答えは、見積もりのまま数える
        usd: answer ? (chatUsd(answer.usage, model) ?? one) : 0,
      };
    }),
  );
  const actual = asked.reduce((sum, { usd }) => sum + usd, 0);
  c.executionCtx.waitUntil(settleBeta(db, code.id, estimate, actual, now));
  const translations = Object.fromEntries(asked.filter((a) => a.translated).map((a) => [a.target, a.translated]));
  return c.json({ translations });
});

/** 文字のやりとりの答え（OpenAI 互換） */
interface Answer {
  choices?: { message?: { content?: unknown } }[];
  usage?: { prompt_tokens?: number; completion_tokens?: number; cost?: unknown };
}

/** 答えの料金。OpenRouter は料金を答えに入れる。ほかは、トークンの数と単価から出す */
function chatUsd(usage: Answer["usage"], model: Model): number | undefined {
  const billed = openrouterUsd(usage);
  if (billed !== undefined || !usage) return billed;
  return usdOf(usage.prompt_tokens ?? 0, model.price.input) + usdOf(usage.completion_tokens ?? 0, model.price.output);
}
