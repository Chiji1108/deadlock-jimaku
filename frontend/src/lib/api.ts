import type { Provider } from "../generated/Provider";
import type { SettingsPatch } from "../generated/SettingsPatch";
import type { SettingsView } from "../generated/SettingsView";

/** エラーのときは本文（サーバーの説明）を投げる */
async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    ...init,
    headers: { "content-type": "application/json", ...init?.headers },
  });
  if (!res.ok) throw new Error((await res.text()) || res.statusText);
  return (await res.json()) as T;
}

export const api = {
  settings: () => request<SettingsView>("/api/settings"),
  updateSettings: (patch: SettingsPatch) =>
    request<SettingsView>("/api/settings", { method: "PATCH", body: JSON.stringify(patch) }),
  /** 空文字列なら消す */
  setKey: (provider: Provider, key: string) =>
    request<SettingsView>(`/api/keys/${provider}`, {
      method: "PUT",
      body: JSON.stringify({ key }),
    }),
  devices: () => request<string[]>("/api/devices"),
  /** 対応している言語のコード */
  languages: () => request<string[]>("/api/languages"),
  demo: () => fetch("/api/demo", { method: "POST" }),
};
