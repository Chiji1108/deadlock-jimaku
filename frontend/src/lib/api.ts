import type { BetaRefusal } from "../generated/BetaRefusal";
import type { Fetched } from "../generated/Fetched";
import type { FontEntry } from "../generated/FontEntry";
import type { GiftStatus } from "../generated/GiftStatus";
import type { Gpus } from "../generated/Gpus";
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
  /** 人の辞書のアイコンに、選んだ画像を保存する */
  savePersonIcon: async (image: Blob) => {
    const res = await fetch("/api/people/icon", { method: "POST", body: image });
    if (!res.ok) throw new Error((await res.text()) || res.statusText);
    return (await res.json()) as Fetched;
  },
  /** 人の辞書のアイコンを、チャンネルの URL か、つないである Steam のアカウントから取ってくる */
  fetchPersonIcon: (from: { url: string } | { steam: true }) =>
    request<Fetched>("/api/people/icon/fetch", { method: "POST", body: JSON.stringify(from) }),
  /** 人の辞書を、ダウンロードのフォルダのファイルに書き出す。書き出した場所を返す */
  exportPeople: () => request<{ path: string }>("/api/people/export", { method: "POST" }),
  /** 書き出したファイルの中身を読み込んで、人の辞書に足す */
  importPeople: (file: string) =>
    request<SettingsView>("/api/people/import", {
      method: "POST",
      body: file,
      headers: { "content-type": "text/plain" },
    }),
  /** PC の中の AI に使える GPU。まだ数えられなければ null */
  gpus: () => request<Gpus | null>("/api/gpus"),
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
  /** ベータのコードをもらって保存する。もらえなければ理由を返す（投げない） */
  joinBeta: async (): Promise<GiftStatus | BetaRefusal> => {
    const res = await fetch("/api/beta", { method: "POST" });
    if (res.ok || res.status === 400) return (await res.json()) as GiftStatus | BetaRefusal;
    throw new Error((await res.text()) || res.statusText);
  },
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
  /** ほかの PC で始めたおすそ分けを、この PC のキーで見る */
  joinShare: () => request<ShareView>("/api/share", { method: "POST" }),
  disableShare: () => request<ShareView>("/api/share", { method: "DELETE" }),
  /** この PC のその会社のキーを預ける（おすそ分けしていなければ、これで始まる） */
  shareKey: (provider: Provider) =>
    request<ShareView>(`/api/share/keys/${provider}`, { method: "POST" }),
  unshareKey: (provider: Provider) =>
    request<ShareView>(`/api/share/keys/${provider}`, { method: "DELETE" }),
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
