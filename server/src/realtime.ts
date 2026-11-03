// Realtime（WebSocket）の中継。アプリと OpenAI の間でメッセージをそのまま渡し、送った音声の長さで料金を数える

import { addUsage, type Code } from "./db";
import { openai } from "./openai";
import { appendSeconds, audioUsd } from "./price";

/** 使われた額を D1 に書く間隔（切れたときにも書く） */
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
  const flush = () => {
    const usd = pending;
    pending = 0;
    flushed = Date.now();
    ctx.waitUntil(addUsage(db, code.id, usd, flushed));
  };

  server.addEventListener("message", (event) => {
    if (typeof event.data === "string") {
      const msg = parse(event.data);
      if (msg?.type === "session.update") {
        model = msg.session?.audio?.input?.transcription?.model;
        // 単価の分からないモデルは使わせない（おすそ分けした人の料金を数えられない）
        if (!model || audioUsd(model, 0) === undefined) {
          server.close(1008, "model not allowed");
          upstream.close(1008, "model not allowed");
          return;
        }
      }
      if (msg?.type === "input_audio_buffer.append" && model && typeof msg.audio === "string") {
        pending += audioUsd(model, appendSeconds(msg.audio)) ?? 0;
        if (Date.now() - flushed >= FLUSH_MS) flush();
      }
    }
    upstream.send(event.data);
  });
  upstream.addEventListener("message", (event) => server.send(event.data));
  const closeBoth = (event: CloseEvent) => {
    flush();
    const code = event.code === 1005 || event.code === 1006 ? 1000 : event.code;
    for (const ws of [server, upstream]) {
      try {
        ws.close(code, event.reason);
      } catch {
        // すでに閉じている
      }
    }
  };
  server.addEventListener("close", closeBoth);
  upstream.addEventListener("close", closeBoth);
  server.addEventListener("error", () => upstream.close(1011));
  upstream.addEventListener("error", () => server.close(1011));

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
