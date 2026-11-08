import type { FontEntry } from "../generated/FontEntry";
import type { GiftStatus } from "../generated/GiftStatus";
import type { Provider } from "../generated/Provider";
import type { RankState } from "../generated/RankState";
import type { SettingsPatch } from "../generated/SettingsPatch";
import type { SettingsView } from "../generated/SettingsView";
import type { ShareView } from "../generated/ShareView";

/** エラーのときは本文（サーバーの説明）を投げる */
async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    ...init,
    headers: { "content-type": "application/json", ...init?.headers },
  });
  if (!res.ok) throw new Error((await res.text()) || res.statusText);
  return (await res.json()) as T;
}

async function post(path: string) {
  const res = await fetch(path, { method: "POST" });
  if (!res.ok) throw new Error((await res.text()) || res.statusText);
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
  /** 選べるフォント */
  fonts: () => request<FontEntry[]>("/api/fonts"),
  /** Google Fonts のフォントを取得する。終わるまで返らない */
  downloadFont: (family: string) => post(`/api/fonts/${encodeURIComponent(family)}/download`),
  demo: () => fetch("/api/demo", { method: "POST" }),
  /** 聞き取りのページを Chrome で開く */
  openChrome: () => post("/api/chrome/open"),
  /** Chrome の専用のデータを消して始め直す */
  resetChrome: () => post("/api/chrome/reset"),
  /** ランクのプレビューの見本。その言語の名前とアイコンは、初めてのときに core が取ってくる */
  rankSample: (lang: string, signal?: AbortSignal) =>
    request<RankState>(`/api/rank/sample?lang=${encodeURIComponent(lang)}`, { signal }),

  /** もらったギフトコードの状態 */
  gift: () => request<GiftStatus>("/api/gift"),
  /** 有効なコードだけ保存される。無効ならその状態を返す（投げない） */
  setGift: async (code: string): Promise<GiftStatus> => {
    const res = await fetch("/api/gift", {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ code }),
    });
    if (res.ok || res.status === 400) return (await res.json()) as GiftStatus;
    throw new Error((await res.text()) || res.statusText);
  },
  removeGift: async () => {
    const res = await fetch("/api/gift", { method: "DELETE" });
    if (!res.ok) throw new Error((await res.text()) || res.statusText);
  },

  /** おすそ分け（自分のキーで人に使ってもらう） */
  share: () => request<ShareView>("/api/share"),
  enableShare: () => request<ShareView>("/api/share", { method: "POST" }),
  disableShare: () => request<ShareView>("/api/share", { method: "DELETE" }),
  /** 自分の xAI のキーも預ける（ギフトコードで Grok を使ってもらう） */
  shareXai: () => request<ShareView>("/api/share/xai", { method: "POST" }),
  unshareXai: () => request<ShareView>("/api/share/xai", { method: "DELETE" }),
  /** `expiresAt` はミリ秒。null は期限なし */
  createCode: (body: { memo: string; expiresAt: number | null; dailyLimitUsd: number }) =>
    request<ShareView>("/api/share/codes", { method: "POST", body: JSON.stringify(body) }),
  updateCode: (
    id: string,
    body: {
      memo?: string;
      expiresAt?: number | null;
      dailyLimitUsd?: number;
      revoked?: boolean;
    },
  ) =>
    request<ShareView>(`/api/share/codes/${encodeURIComponent(id)}`, {
      method: "PATCH",
      body: JSON.stringify(body),
    }),
};
