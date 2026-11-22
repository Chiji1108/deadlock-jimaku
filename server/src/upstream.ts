// QwenCloud と OpenRouter、Perplexity への通信（テストではこのモジュールを差し替える）。ベータ期間中、開発者のキーで中継する（DESIGN.md 17.2）

/** QwenCloud のシンガポールの口 */
const QWEN = "https://ap-southeast-1.maas.qwencloudapi.com";
const OPENROUTER = "https://openrouter.ai/api/v1";
const PERPLEXITY = "https://api.perplexity.ai";

function send(url: string, key: string, body: unknown, signal?: AbortSignal): Promise<Response> {
  return fetch(url, {
    method: "POST",
    headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
}

/** 文字のやりとりを頼む（/v2 の仕事。モデルと頼み方、待つ時間は tasks.ts が決める） */
export function chat(provider: "qwen" | "openrouter", key: string, body: unknown, timeoutMs: number): Promise<Response> {
  const base = provider === "qwen" ? `${QWEN}/compatible-mode/v1` : OPENROUTER;
  return send(`${base}/chat/completions`, key, body, AbortSignal.timeout(timeoutMs));
}

/** 送り直す前に待つ */
export const wait = (ms: number) => new Promise<void>((done) => setTimeout(done, ms));

/** `timeoutMs` を渡すと、その時間で待つのをやめる（/v2 の仕事。/v1 の中継は、アプリが待つのをやめる） */
const until = (timeoutMs?: number) => (timeoutMs === undefined ? undefined : AbortSignal.timeout(timeoutMs));

export function qwen(path: string, key: string, body: unknown, timeoutMs?: number): Promise<Response> {
  return send(`${QWEN}${path}`, key, body, until(timeoutMs));
}

/** 途中の字幕の WebSocket を開く。開けたら、応答の `webSocket` に入っている */
export function qwenLive(key: string): Promise<Response> {
  return fetch(`${QWEN}/api-ws/v1/inference`, { headers: { Upgrade: "websocket", Authorization: `bearer ${key}` } });
}

export function openrouter(path: string, key: string, body: unknown, timeoutMs?: number): Promise<Response> {
  return send(`${OPENROUTER}${path}`, key, body, until(timeoutMs));
}

export function perplexity(path: string, key: string, body: unknown, timeoutMs?: number): Promise<Response> {
  return send(`${PERPLEXITY}${path}`, key, body, until(timeoutMs));
}
