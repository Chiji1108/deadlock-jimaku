// Qwen の途中の字幕（WebSocket）の中継。ベータのコードで、開発者のキーを使う（DESIGN.md 17.2）。
// アプリと Qwen の間でメッセージを渡す。アプリが送る形（`run-task`、音声のバイナリ、`finish-task`）だけを通し、
// `run-task` は作り直して送る（モデルやパラメータを足させない）。
// 料金は、音声 30 秒分ごとに見積もりを先に足し（上限に届いていたら切る）、Qwen が知らせる使用量で実際の額に直す

import { keyFailed, reserveBeta, settleBeta, type RelayKey } from "./beta";
import type { Db } from "./db";
import { QWEN_LIVE } from "./price";
import { qwenLive } from "./upstream";

/** アプリが送る音声（16kHz・16bit の PCM）の 1 秒のバイト数 */
const BYTES_PER_SECOND = 32_000;
/** 見積もりを足し直す間隔（送られた音声の長さ）。実時間より速く送られても、この長さごとに上限を確かめる */
const WINDOW_SECONDS = 30;
/** 音声 1 秒の見積もり（ドル）。実測は入力約 11 トークン（DESIGN.md 3.4）。出力を含めて倍を見ておく */
const SECOND_USD = (2 * (11 * QWEN_LIVE.input + 3 * QWEN_LIVE.output)) / 1e6;
const WINDOW_USD = WINDOW_SECONDS * SECOND_USD;
/** キーのせいで断られたときの番号（ほかのキーなら通るかもしれない） */
const KEY_TROUBLE = [401, 402, 403, 429];
/** `task-failed` のうち、キーのせいのもの（使えない、残高がない、回数の上限） */
const KEY_FAILURE = /ApiKey|Unauthorized|AccessDenied|Arrearage|Throttling|Quota/i;

interface Ctx {
  waitUntil(promise: Promise<unknown>): void;
}

/** 断るときの応答。`limit` は今日の上限、`upstream` は開発者のキーや Qwen の不調 */
export type Refused = { error: "limit" | "upstream" };

export async function relayQwenLive(db: Db, codeId: string, keys: RelayKey[], ctx: Ctx): Promise<Response | Refused> {
  if (!(await reserveBeta(db, codeId, WINDOW_USD, Date.now()))) return { error: "limit" };
  const release = () => ctx.waitUntil(settleBeta(db, codeId, WINDOW_USD, 0, Date.now()));
  // キーが断られたら、次のキーに回す（`relayed` と同じ決まり）
  let upstream: WebSocket | null = null;
  let used: RelayKey | undefined;
  for (const key of keys) {
    const secret = await key.key().catch(() => undefined);
    const res = secret === undefined ? undefined : await qwenLive(secret).catch(() => undefined);
    upstream = res?.webSocket ?? null;
    used = key;
    if (upstream) break;
    void res?.body?.cancel();
    const trouble = secret === undefined ? 0 : res && KEY_TROUBLE.includes(res.status) ? res.status : undefined;
    if (trouble === undefined) break;
    if (key.id) ctx.waitUntil(keyFailed(db, key.id, trouble, Date.now()));
  }
  if (!upstream) {
    release();
    return { error: "upstream" };
  }
  upstream.accept();
  const { 0: client, 1: server } = new WebSocketPair();
  server.accept();
  // 音声は ArrayBuffer で受ける（既定の Blob は、長さを数えるにも渡すにも読み出しが要る）
  server.binaryType = "arraybuffer";
  upstream.binaryType = "arraybuffer";

  let task: string | undefined;
  /** 今の見積もりの間に送られた音声（秒） */
  let seconds = 0;
  /** Qwen が知らせた使用量の額（つないでからの合計）と、もう数えた分 */
  let usage: number | undefined;
  let counted = 0;
  let writing: Promise<void> = Promise.resolve();
  let closed = false;

  /** 今の見積もりを実際の額に直す。使用量がまだ届いていなければ、音声の長さで数える */
  const actual = () => {
    const usd = usage === undefined ? seconds * SECOND_USD : Math.max(usage - counted, 0);
    counted = usage ?? counted;
    seconds = 0;
    return usd;
  };
  /** 先に足してある見積もり。切るときと足し直すときに、実際の額に直す */
  let reserved = WINDOW_USD;
  const shut = (status: number, reason: string) => {
    closed = true;
    for (const ws of [server, upstream]) {
      try {
        ws.close(status, reason);
      } catch {
        // すでに閉じている
      }
    }
  };
  /** 先に足した見積もりを、実際の額に直す（順番に書く） */
  const settle = () => {
    const [estimate, usd] = [reserved, actual()];
    reserved = 0;
    return settleBeta(db, codeId, estimate, usd, Date.now());
  };
  const close = (status: number, reason: string) => {
    if (closed) return;
    shut(status, reason);
    writing = writing.then(settle);
    ctx.waitUntil(writing);
  };
  /** 次の 30 秒分を先に足す。上限に届いていたら切る（張り直しても、つなぐ前に断る） */
  const renew = () => {
    writing = writing.then(async () => {
      await settle();
      if (closed) return;
      if (await reserveBeta(db, codeId, WINDOW_USD, Date.now())) reserved = WINDOW_USD;
      else shut(1008, "limit");
    });
    ctx.waitUntil(writing);
  };
  const send = (ws: WebSocket, data: string | ArrayBuffer) => {
    if (!closed && ws.readyState === WebSocket.OPEN) ws.send(data);
  };
  const header = (action: string) => ({ action, task_id: task, streaming: "duplex" });

  server.addEventListener("message", (event) => {
    if (typeof event.data !== "string") {
      if (!task) return close(1008, "run-task comes first");
      seconds += (event.data as ArrayBuffer).byteLength / BYTES_PER_SECOND;
      if (seconds >= WINDOW_SECONDS) renew();
      return send(upstream, event.data);
    }
    const msg = parse(event.data);
    const action = msg?.header?.action;
    if (action === "run-task" && !task) {
      const id = msg?.header?.task_id;
      const hints = msg?.payload?.parameters?.language_hints;
      const language = Array.isArray(hints) && hints.length === 1 ? hints[0] : undefined;
      if (typeof id !== "string" || !/^[0-9a-f]{1,64}$/i.test(id)) return close(1008, "message not allowed");
      if (msg?.payload?.model !== QWEN_LIVE.model) return close(1008, "model not allowed");
      if (typeof language !== "string" || language.length > 8) return close(1008, "message not allowed");
      task = id;
      const payload = {
        task_group: "audio",
        task: "asr",
        function: "recognition",
        model: QWEN_LIVE.model,
        parameters: { format: "pcm", sample_rate: 16_000, language_hints: [language] },
        input: {},
      };
      return send(upstream, JSON.stringify({ header: header("run-task"), payload }));
    }
    if (action === "finish-task" && task) {
      return send(upstream, JSON.stringify({ header: header("finish-task"), payload: { input: {} } }));
    }
    close(1008, "message not allowed");
  });
  upstream.addEventListener("message", (event) => {
    const msg = typeof event.data === "string" ? parse(event.data) : undefined;
    if (msg?.header?.event === "result-generated") {
      const { input_tokens: input, output_tokens: output } = msg.payload?.usage ?? {};
      if (typeof input === "number" && typeof output === "number") {
        usage = (input * QWEN_LIVE.input + output * QWEN_LIVE.output) / 1e6;
      }
    }
    // 開発者のキーが断られた。利用者には関係がないので、中身を渡さずに切る（アプリは張り直し、次は別のキーを使う）
    if (msg?.header?.event === "task-failed" && KEY_FAILURE.test(String(msg.header.error_code))) {
      if (used?.id) ctx.waitUntil(keyFailed(db, used.id, 401, Date.now()));
      return close(1011, "upstream");
    }
    send(server, event.data);
  });
  const passClose = (event: CloseEvent) =>
    close(event.code === 1005 || event.code === 1006 ? 1000 : event.code, event.reason);
  server.addEventListener("close", passClose);
  upstream.addEventListener("close", passClose);
  server.addEventListener("error", () => close(1011, "client error"));
  upstream.addEventListener("error", () => close(1011, "upstream error"));

  return new Response(null, { status: 101, webSocket: client });
}

interface Message {
  header?: { action?: unknown; event?: unknown; task_id?: unknown; error_code?: unknown };
  payload?: {
    model?: unknown;
    parameters?: { language_hints?: unknown };
    usage?: { input_tokens?: unknown; output_tokens?: unknown };
  };
}

function parse(text: string): Message | undefined {
  try {
    return (JSON.parse(text) ?? undefined) as Message | undefined;
  } catch {
    return undefined;
  }
}
