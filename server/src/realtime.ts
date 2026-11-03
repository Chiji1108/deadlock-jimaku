// Realtime（WebSocket）の中継。アプリと OpenAI の間でメッセージをそのまま渡し、送った音声の長さで料金を数える。
// 単価の分かるモデルを session.update で決めるまでは、ほかのメッセージを通さない（数えられない音声を送らせない）

import { addUsage, codeById, usable, type Code } from "./db";
import { openai } from "./openai";
import { appendSeconds, audioUsd } from "./price";

/** 使われた額を D1 に書き、コードがまだ使えるか確かめる間隔（切れたときにも書く） */
const FLUSH_MS = 30_000;

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
    flush();
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
    ctx.waitUntil(addUsage(db, code.id, usd, flushed));
  };
  // 書くついでに、止められた・猶予も過ぎたコードなら切る（配信の途中でも「止める」はすぐ効かせる）
  const flushAndCheck = () => {
    flush();
    ctx.waitUntil(
      codeById(db, code.id).then((now) => {
        if (!now || !usable(now, Date.now())) close(1008, "gift code is no longer usable");
      }),
    );
  };
  const send = (ws: WebSocket, data: string | ArrayBuffer) => {
    if (!closed && ws.readyState === WebSocket.OPEN) ws.send(data);
  };

  server.addEventListener("message", (event) => {
    const msg = typeof event.data === "string" ? parse(event.data) : undefined;
    if (msg?.type === "session.update") {
      model = msg.session?.audio?.input?.transcription?.model;
      // 単価の分からないモデルは使わせない（おすそ分けした人の料金を数えられない）
      if (!model || audioUsd(model, 0) === undefined) return close(1008, "model not allowed");
    } else if (!model) {
      return close(1008, "session.update with a model comes first");
    }
    if (msg?.type === "input_audio_buffer.append" && typeof msg.audio === "string") {
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
  session?: { audio?: { input?: { transcription?: { model?: string } } } };
}

function parse(text: string): ClientEvent | undefined {
  try {
    return JSON.parse(text) as ClientEvent;
  } catch {
    return undefined;
  }
}
