// QwenCloud と OpenRouter への通信（テストではこのモジュールを差し替える）。ベータの間、開発者のキーで中継する（DESIGN.md 17.2）

/** QwenCloud のシンガポールの口 */
const QWEN = "https://ap-southeast-1.maas.qwencloudapi.com";
const OPENROUTER = "https://openrouter.ai/api/v1";

function send(url: string, key: string, body: unknown): Promise<Response> {
  return fetch(url, {
    method: "POST",
    headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function qwen(path: string, key: string, body: unknown): Promise<Response> {
  return send(`${QWEN}${path}`, key, body);
}

/** 途中の字幕の WebSocket を開く。開けたら、応答の `webSocket` に入っている */
export function qwenLive(key: string): Promise<Response> {
  return fetch(`${QWEN}/api-ws/v1/inference`, { headers: { Upgrade: "websocket", Authorization: `bearer ${key}` } });
}

export function openrouter(path: string, key: string, body: unknown): Promise<Response> {
  return send(`${OPENROUTER}${path}`, key, body);
}
