// 操作パネルの状態。各タブはこれを読み、変更は api 経由でサーバーに送る（サーバーの返す設定で置き換える）
import { toast } from "svelte-sonner";
import type { Provider } from "../generated/Provider";
import type { ServerMsg } from "../generated/ServerMsg";
import type { SettingsPatch } from "../generated/SettingsPatch";
import type { SettingsView } from "../generated/SettingsView";
import type { MicError } from "../generated/MicError";
import type { TranscriptionError } from "../generated/TranscriptionError";
import type { UsageView } from "../generated/UsageView";
import { api } from "../lib/api";
import { connect, type Status } from "../lib/ws";

/** OBS に貼る URL（axum。開発中に Vite から開いても同じ） */
export const overlayUrl = (name: "subtitles" | "rank" | "items") =>
  `${import.meta.env.CORE_ORIGIN}/overlay/${name}`;

/** 言語名は画面の言語（日本語）で出す。翻訳先は配信者が読める名前で選ぶ */
const languageNames = new Intl.DisplayNames("ja", { type: "language" });
/** Intl の名前が不自然なもの（「ポルトガル語 (イベリア半島)」）は地域を外す */
const NAME_CODE: Record<string, string> = { "pt-PT": "pt" };

/** これより小さい入力が続いたら「音が届いていない」 */
const SILENT_DB = -60;
const SILENT_MS = 5000;

class Panel {
  view = $state<SettingsView | undefined>();
  /** 対応している言語のコード */
  languages = $state<string[]>([]);
  /** 取れるまでは undefined */
  devices = $state<string[] | undefined>();
  server = $state<Status>("connecting");
  /** 字幕を始められなかった・止まった理由 */
  transcriptionError = $state<TranscriptionError | undefined>();
  /** 直近の入力レベル（dBFS）。字幕を動かしている間だけ届く */
  level = $state<number | undefined>();
  /** OpenAI の使用量と概算の料金 */
  usage = $state<UsageView | undefined>();
  /** 最後に SILENT_DB を超えた時刻 */
  private heardAt = 0;
  /** 最後にレベルが届いた時刻 */
  private levelAt = 0;
  now = $state(Date.now());

  get settings() {
    return this.view?.settings;
  }

  /** OpenAI の API キーがまだ保存されていない（読めないだけのときは含めない） */
  get keyMissing() {
    return this.view?.keys.openai.state === "missing";
  }

  /** 字幕を始める操作をしていて、止まった理由もない */
  get running() {
    return !!this.settings?.transcriptionEnabled && !this.transcriptionError;
  }

  /** 動いているのに、しばらく音が来ていない */
  get silent() {
    return this.running && this.level !== undefined && this.now - this.heardAt > SILENT_MS;
  }

  label = (code: string) => languageNames.of(NAME_CODE[code] ?? code) ?? code;

  start() {
    const tick = setInterval(() => (this.now = Date.now()), 500);
    void this.run(async () => {
      [this.view, this.languages] = await Promise.all([api.settings(), api.languages()]);
    });
    void this.refreshDevices();
    const disconnect = connect(
      ["status", "level", "usage"],
      (msg) => this.onMessage(msg),
      (s) => (this.server = s),
    );
    return () => {
      clearInterval(tick);
      disconnect();
    };
  }

  private onMessage(msg: ServerMsg) {
    if (msg.type === "transcription.status") {
      this.transcriptionError = msg.error ?? undefined;
    } else if (msg.type === "usage") {
      this.usage = msg.usage;
    } else if (msg.type === "audio.level") {
      // 届き始めた（張り直しで途切れた後も）時点を「聞こえた」とみなし、そこから数える
      const now = Date.now();
      if (now - this.levelAt > 1000 || msg.db > SILENT_DB) this.heardAt = now;
      this.levelAt = now;
      this.level = msg.db;
    }
  }

  async refreshDevices() {
    this.devices = await api.devices().catch(() => []);
  }

  update = (patch: SettingsPatch) =>
    this.run(async () => {
      this.view = await api.updateSettings(patch);
    });

  setTranscription(enabled: boolean) {
    this.level = undefined;
    return this.update({ transcriptionEnabled: enabled });
  }

  /** 保存できたら true */
  setKey = (provider: Provider, key: string) =>
    this.run(async () => {
      this.view = await api.setKey(provider, key);
      toast.success(key ? "保存しました" : "削除しました");
    });

  demo = () =>
    this.run(async () => {
      const res = await api.demo();
      if (!res.ok) throw new Error(await res.text());
    });

  /** 失敗したらサーバーの説明を出す。成功したら true */
  private async run(action: () => Promise<void>) {
    try {
      await action();
      return true;
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
      return false;
    }
  }
}

export const panel = new Panel();

/** 字幕が止まった理由を文にする（サーバーは種類と中身だけを返す） */
export function transcriptionErrorText(e: TranscriptionError): string {
  switch (e.code) {
    case "missingKey":
      return "OpenAI の API キーが設定されていません";
    case "inputEnded":
      return "マイクの入力が途切れました";
    case "microphone":
      return micErrorText(e);
    case "failed":
      return `エラーで止まりました: ${e.detail}`;
    case "crashed":
      return `内部エラーで止まりました: ${e.detail}`;
    default:
      return unknown(e);
  }
}

function micErrorText(e: MicError): string {
  switch (e.problem) {
    case "listFailed":
      return `マイクの一覧を取れません（${e.detail}）`;
    case "noDevices":
      return "マイクが見つかりません。マイクをつないでください";
    case "notFound":
      return `マイク「${e.name}」が見つかりません`;
    case "noDefault":
      return "既定のマイクがありません";
    default:
      return unknown(e);
  }
}

/** 種類を足したら、ここで型エラーになる（版の違うサーバーから届いたときは中身を出す） */
const unknown = (e: never) => `止まりました（${JSON.stringify(e)}）`;
