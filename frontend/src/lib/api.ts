import type { LiveStatus } from "../generated/LiveStatus";
import type { GiftStatus } from "../generated/GiftStatus";
import type { LiveText } from "../generated/LiveText";
import type { Provider } from "../generated/Provider";
import type { Segment } from "../generated/Segment";
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

/** 閉じる途中のページからも届くよう keepalive にする */
const post = (path: string, body: unknown) =>
  fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    keepalive: true,
  });

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
  /** 閉じられた Chrome を開き直す */
  openChrome: () => post("/api/chrome/open", {}),
  /** Chrome のページの状態を知らせる */
  liveStatus: (status: LiveStatus) => post("/api/live/status", status),
  /** ハイブリッドモードの途中の文を送る */
  live: (live: LiveText) => post("/api/live", live),
  /** Chrome のページが認識した字幕を送る */
  segment: (segment: Segment) => post("/api/segments", segment),

  /** もらったギフトコードの状態 */
  gift: () => request<GiftStatus>("/api/gift"),
  /** 使えるコードだけ保存される。使えなければその状態を返す（投げない） */
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
  /** `expiresAt` はミリ秒。null は期限なし */
  createCode: (body: { memo: string; expiresAt: number | null }) =>
    request<ShareView>("/api/share/codes", { method: "POST", body: JSON.stringify(body) }),
  updateCode: (id: string, body: { memo?: string; expiresAt?: number | null; revoked?: boolean }) =>
    request<ShareView>(`/api/share/codes/${encodeURIComponent(id)}`, {
      method: "PATCH",
      body: JSON.stringify(body),
    }),
};
