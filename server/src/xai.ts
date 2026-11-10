// xAI への通信（テストではこのモジュールを差し替える）

const ORIGIN = "https://api.x.ai";

export function xai(path: string, key: string, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers);
  headers.set("Authorization", `Bearer ${key}`);
  return fetch(`${ORIGIN}${path}`, { ...init, headers });
}

/** キーが使えるか（モデルの一覧を取れるか。間違ったキーには 401 ではなく 400 が返る） */
export async function validXaiKey(key: string): Promise<boolean> {
  const res = await xai("/v1/models", key);
  return res.ok;
}
