// Qwen の途中の字幕（WebSocket）の中継。ベータのコードで、開発者のキーを使う（DESIGN.md 17.2）。
// アプリと Qwen の間でメッセージを渡す。アプリが送る形（`run-task`、音声のバイナリ、`finish-task`）だけを通し、
// `run-task` は作り直して送る（モデルやパラメータを足させない）。
// 料金は、音声 30 秒分ごとに見積もりを先に足し（上限に届いていたら切る）、Qwen が知らせる使用量で実際の額に直す。
//
// 仕事の口（/v2/live。DESIGN.md 17.4）では、Qwen とのやりとりを Worker が持つ（`language` を渡す）。アプリは音声を流し、
// `{"type":"finish"}` で終える。Worker は、聞き始めたら `{"type":"ready"}`、文が届くたびに
// `{"type":"sentence","id","begin","text","words":[{"at","text"}]}`（時刻は、流した音声の先頭からのミリ秒）を送る

import { betaClosed, keyFailed, reasonOf, reserveBeta, settleBeta, type RelayKey } from "./beta";
import { codeById, refusal, type Db } from "./db";
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

export async function relayQwenLive(
  db: Db,
  codeId: string,
  keys: RelayKey[],
  ctx: Ctx,
  language?: string,
): Promise<Response | Refused> {
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
    const trouble = secret === undefined ? 0 : res && KEY_TROUBLE.includes(res.status) ? res.status : undefined;
    if (trouble === undefined) {
      void res?.body?.cancel();
      break;
    }
    const reason = await reasonOf(res);
    if (key.id) ctx.waitUntil(keyFailed(db, key.id, trouble, reason, Date.now()));
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

  /** Qwen に頼んだ仕事の名前。/v2 では Worker が付けて、すぐに頼む */
  let task: string | undefined;
  /** 音声を流してよい。/v1 では `run-task` を通したら、/v2 では Qwen が聞き始めたら */
  let ready = false;
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
  /** 足し直している間に届いた音声では、もう 1 度足し直さない */
  let renewing = false;
  /**
   * 次の 30 秒分を先に足す。上限に届いていたら切る（張り直しても、つなぐ前に断る）。
   * つないだあとに止められたコードと、閉じたベータも、ここで切る（ほかの中継は要求ごとに確かめる）
   */
  const renew = () => {
    if (renewing) return;
    renewing = true;
    writing = writing.then(async () => {
      await settle();
      renewing = false;
      if (closed) return;
      // 上限は、下で足すときに確かめる
      const code = await codeById(db, codeId);
      const refused = code ? refusal(code, Date.now()) : "revoked";
      if ((refused && refused !== "limit") || (await betaClosed(db))) {
        return shut(1008, "code is no longer usable");
      }
      if (await reserveBeta(db, codeId, WINDOW_USD, Date.now())) reserved = WINDOW_USD;
      else shut(1008, "limit");
    });
    ctx.waitUntil(writing);
  };
  const send = (ws: WebSocket, data: string | ArrayBuffer) => {
    if (!closed && ws.readyState === WebSocket.OPEN) ws.send(data);
  };
  const header = (action: string) => ({ action, task_id: task, streaming: "duplex" });
  const run = (language: string) => {
    const payload = {
      task_group: "audio",
      task: "asr",
      function: "recognition",
      model: QWEN_LIVE.model,
      parameters: { format: "pcm", sample_rate: 16_000, language_hints: [language] },
      input: {},
    };
    send(upstream, JSON.stringify({ header: header("run-task"), payload }));
  };
  const finish = () => send(upstream, JSON.stringify({ header: header("finish-task"), payload: { input: {} } }));
  if (language !== undefined) {
    task = crypto.randomUUID().replaceAll("-", "");
    run(language);
  }

  server.addEventListener("message", (event) => {
    if (typeof event.data !== "string") {
      if (!ready) return close(1008, language === undefined ? "run-task comes first" : "wait for ready");
      seconds += (event.data as ArrayBuffer).byteLength / BYTES_PER_SECOND;
      if (seconds >= WINDOW_SECONDS) renew();
      return send(upstream, event.data);
    }
    const msg = parse(event.data);
    if (language !== undefined) return msg?.type === "finish" ? finish() : close(1008, "message not allowed");
    const action = msg?.header?.action;
    if (action === "run-task" && !task) {
      const id = msg?.header?.task_id;
      const hints = msg?.payload?.parameters?.language_hints;
      const hinted = Array.isArray(hints) && hints.length === 1 ? hints[0] : undefined;
      if (typeof id !== "string" || !/^[0-9a-f]{1,64}$/i.test(id)) return close(1008, "message not allowed");
      if (msg?.payload?.model !== QWEN_LIVE.model) return close(1008, "model not allowed");
      if (typeof hinted !== "string" || hinted.length > 8) return close(1008, "message not allowed");
      task = id;
      ready = true;
      return run(hinted);
    }
    if (action === "finish-task" && task) return finish();
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
      // 番号は、HTTP で断られたときに合わせる（回数の上限は 429、それ以外はキーが使えない 401）
      const code = String(msg.header.error_code);
      const status = /Throttling|Quota/i.test(code) ? 429 : 401;
      const message = typeof msg.header.error_message === "string" ? msg.header.error_message : "";
      if (used?.id) ctx.waitUntil(keyFailed(db, used.id, status, [code, message].filter(Boolean).join(": "), Date.now()));
      return close(1011, "upstream");
    }
    if (language === undefined) return send(server, event.data);
    // Qwen の形を、仕事の口の形に直す。知らない知らせは渡さない
    switch (msg?.header?.event) {
      case "task-started":
        ready = true;
        return send(server, JSON.stringify({ type: "ready" }));
      case "result-generated": {
        const sentence = spoken(msg.payload?.output?.sentence);
        return sentence && send(server, JSON.stringify(sentence));
      }
      case "task-failed":
        return close(1011, "failed");
      case "task-finished":
        return close(1000, "finished");
    }
  });
  // /v2 では、Qwen が書いた理由を渡さない（外の不調の中身は、利用者に関係がない）
  const passClose = (event: CloseEvent) =>
    close(event.code === 1005 || event.code === 1006 ? 1000 : event.code, language === undefined ? event.reason : "closed");
  server.addEventListener("close", passClose);
  upstream.addEventListener("close", passClose);
  server.addEventListener("error", () => close(1011, "client error"));
  upstream.addEventListener("error", () => close(1011, "upstream error"));

  return new Response(null, { status: 101, webSocket: client });
}

/** Qwen の文を、仕事の口の形にする。句読点は、その文字の後ろにつなげる */
function spoken(sentence: Sentence | undefined) {
  if (typeof sentence?.sentence_id !== "number") return undefined;
  const text = (value: unknown) => (typeof value === "string" ? value : "");
  const ms = (value: unknown) => (typeof value === "number" ? value : undefined);
  const words = Array.isArray(sentence.words) ? sentence.words : [];
  return {
    type: "sentence",
    id: sentence.sentence_id,
    begin: ms(sentence.begin_time),
    text: text(sentence.text),
    words: words.map((w) => ({ at: ms(w?.begin_time), text: text(w?.text) + text(w?.punctuation) })),
  };
}

interface Sentence {
  sentence_id?: unknown;
  begin_time?: unknown;
  text?: unknown;
  words?: { begin_time?: unknown; text?: unknown; punctuation?: unknown }[];
}

interface Message {
  /** /v2 のアプリが送る */
  type?: unknown;
  header?: { action?: unknown; event?: unknown; task_id?: unknown; error_code?: unknown; error_message?: unknown };
  payload?: {
    model?: unknown;
    parameters?: { language_hints?: unknown };
    usage?: { input_tokens?: unknown; output_tokens?: unknown };
    output?: { sentence?: Sentence };
  };
}

function parse(text: string): Message | undefined {
  try {
    return (JSON.parse(text) ?? undefined) as Message | undefined;
  } catch {
    return undefined;
  }
}
