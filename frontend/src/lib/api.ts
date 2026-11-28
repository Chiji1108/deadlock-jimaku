import type { ScreenStatus } from "../generated/ScreenStatus";
import type { BetaRefusal } from "../generated/BetaRefusal";
import type { CodeStatus } from "../generated/CodeStatus";
import type { Fetched } from "../generated/Fetched";
import type { FontEntry } from "../generated/FontEntry";
import type { Gpus } from "../generated/Gpus";
import type { Mic } from "../generated/Mic";
import type { Provider } from "../generated/Provider";
import type { Purchase } from "../generated/Purchase";
import type { RankState } from "../generated/RankState";
import type { ServerModel } from "../generated/ServerModel";
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

async function post(path: string, method = "POST") {
  const res = await fetch(path, { method });
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
  /** OBS の WebSocket サーバーのパスワード。空文字列なら消す */
  setObsPassword: (key: string) =>
    request<SettingsView>("/api/obs/password", { method: "PUT", body: JSON.stringify({ key }) }),
  /** OBS のソースの名前。つながらなければ、その理由 */
  obsInputs: async (): Promise<string[] | ScreenStatus> => (await fetch("/api/obs/inputs")).json(),
  devices: () => request<Mic[]>("/api/devices"),
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
  /** OBS の買ったアイテムの表示に、この買い物を 1 回出す（テスト表示） */
  showPurchase: (purchase: Purchase) =>
    fetch("/api/purchases/demo", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(purchase),
    }),
  /** 聞き取りのページを Chrome で開く */
  openChrome: () => post("/api/chrome/open"),
  /** Chrome の専用のデータを消して始め直す */
  resetChrome: () => post("/api/chrome/reset"),
  /** 統計の回を 1 つ消す */
  forgetSession: (id: number) => post(`/api/stats/sessions/${id}`, "DELETE"),
  /** 統計をすべて消す */
  forgetStats: () => post("/api/stats", "DELETE"),
  /** ランクのプレビューの見本。その言語の名前とアイコンは、初めてのときに core が取ってくる */
  rankSample: (lang: string, signal?: AbortSignal) =>
    request<RankState>(`/api/rank/sample?lang=${encodeURIComponent(lang)}`, { signal }),

  /** 新しい版に入れ替えて再起動する（デスクトップアプリが行う。終わる前に返る） */
  updateApp: () => post("/api/update"),

  /** デドロ字幕のサーバーが、仕事ごとに今使っているモデル */
  serverModels: () => request<Record<string, ServerModel>>("/api/server/models"),

  /** ベータのコードをもらって保存する。もらえなければ理由を返す（投げない） */
  joinBeta: async (): Promise<CodeStatus | BetaRefusal> => {
    const res = await fetch("/api/beta", { method: "POST" });
    if (res.ok || res.status === 400) return (await res.json()) as CodeStatus | BetaRefusal;
    throw new Error((await res.text()) || res.statusText);
  },
};
