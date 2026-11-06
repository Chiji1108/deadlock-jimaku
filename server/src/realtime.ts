// Realtime（WebSocket）の中継。アプリと OpenAI の間でメッセージをそのまま渡し、送った音声の長さで料金を数える。
// 数えられない音声を送らせないよう、アプリが送る種類の文字のメッセージだけを通し、
// 単価の分かるモデルと音声の形を session.update で決めるまでは、ほかのメッセージを通さない

import { addUsage, codeById, refusal, type Code } from "./db";
import { openai } from "./openai";
import { appendSeconds, audioUsd, REALTIME_RATE } from "./price";

/** 使われた額を D1 に書き、コードがまだ使えるか確かめる間隔（切れたときにも書く） */
const FLUSH_MS = 30_000;

/** アプリが送るメッセージの種類 */
const CLIENT_EVENTS = new Set([
  "session.update",
  "input_audio_buffer.append",
  "input_audio_buffer.commit",
  "input_audio_buffer.clear",
]);

export async function relay(
  db: D1Database,
  code: Code,
  key: string,
  ctx: { waitUntil(promise: Promise<unknown>): void },
): Promise<Response> {
  const res = await openai("/v1/realtime?intent=transcription", key, {
    headers: { Upgrade: "websocket" },
  });
  const upstream = res.webSocket;
  if (!upstream) return new Response(await res.text(), { status: res.status || 502 });
  upstream.accept();
  const { 0: client, 1: server } = new WebSocketPair();
  server.accept();

  let model: string | undefined;
  let pending = 0;
  let flushed = Date.now();
  let closed = false;
  const close = (status: number, reason: string) => {
    if (closed) return;
    closed = true;
    ctx.waitUntil(flush());
    for (const ws of [server, upstream]) {
      try {
        ws.close(status, reason);
      } catch {
        // すでに閉じている
      }
    }
  };
  const flush = () => {
    const usd = pending;
    pending = 0;
    flushed = Date.now();
    return addUsage(db, code.id, usd, flushed);
  };
  // 書いてから、止められた・猶予も過ぎた・今日の上限に達したコードなら切る
  // （配信の途中でも「止める」と上限はすぐ効かせる）
  const flushAndCheck = () => {
    ctx.waitUntil(
      flush()
        .then(() => codeById(db, code.id))
        .then((now) => {
          if (!now || refusal(now, Date.now())) close(1008, "gift code is no longer usable");
        }),
    );
  };
  const send = (ws: WebSocket, data: string | ArrayBuffer) => {
    if (!closed && ws.readyState === WebSocket.OPEN) ws.send(data);
  };

  server.addEventListener("message", (event) => {
    const msg = typeof event.data === "string" ? parse(event.data) : undefined;
    if (!msg?.type || !CLIENT_EVENTS.has(msg.type)) return close(1008, "message not allowed");
    if (msg.type === "session.update") {
      const input = msg.session?.audio?.input;
      // 長さを数えられない音声は使わせない。format を省いた形はアプリが送らないので受けず、
      // 省いたときの形を OpenAI の既定に任せない
      if (
        msg.session?.type !== "transcription" ||
        input?.format?.type !== "audio/pcm" ||
        input.format.rate !== REALTIME_RATE
      ) {
        return close(1008, "audio format not allowed");
      }
      model = input.transcription?.model;
      // 単価の分からないモデルは使わせない（おすそ分けした人の料金を数えられない）
      if (!model || audioUsd(model, 0) === undefined) return close(1008, "model not allowed");
    } else if (!model) {
      return close(1008, "session.update with a model comes first");
    } else if (msg.type === "input_audio_buffer.append") {
      if (typeof msg.audio !== "string") return close(1008, "message not allowed");
      pending += audioUsd(model, appendSeconds(msg.audio)) ?? 0;
    }
    if (Date.now() - flushed >= FLUSH_MS) flushAndCheck();
    send(upstream, event.data);
  });
  upstream.addEventListener("message", (event) => send(server, event.data));
  const passClose = (event: CloseEvent) =>
    close(event.code === 1005 || event.code === 1006 ? 1000 : event.code, event.reason);
  server.addEventListener("close", passClose);
  upstream.addEventListener("close", passClose);
  server.addEventListener("error", () => close(1011, "client error"));
  upstream.addEventListener("error", () => close(1011, "upstream error"));

  return new Response(null, { status: 101, webSocket: client });
}

interface ClientEvent {
  type?: string;
  audio?: unknown;
  session?: {
    type?: string;
    audio?: { input?: { format?: { type?: string; rate?: number }; transcription?: { model?: string } } };
  };
}

function parse(text: string): ClientEvent | undefined {
  try {
    return (JSON.parse(text) ?? undefined) as ClientEvent | undefined;
  } catch {
    return undefined;
  }
}
