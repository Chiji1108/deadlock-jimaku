// OpenAI への通信（テストではこのモジュールを差し替える）

const ORIGIN = "https://api.openai.com";

export function openai(path: string, key: string, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers);
  headers.set("Authorization", `Bearer ${key}`);
  return fetch(`${ORIGIN}${path}`, { ...init, headers });
}

/** キーが使えるか（モデルの一覧を取れるか） */
export async function validKey(key: string): Promise<boolean> {
  const res = await openai("/v1/models", key);
  return res.ok;
}
