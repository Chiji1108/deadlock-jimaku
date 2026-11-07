// 操作パネルの状態。各タブはこれを読み、変更は api 経由でサーバーに送る（サーバーの返す設定で置き換える）
import { toast } from "svelte-sonner";
import type { LiveStatus } from "../generated/LiveStatus";
import type { FontEntry } from "../generated/FontEntry";
import type { GiftStatus } from "../generated/GiftStatus";
import type { Provider } from "../generated/Provider";
import type { ServerMsg } from "../generated/ServerMsg";
import type { SettingsPatch } from "../generated/SettingsPatch";
import type { SettingsView } from "../generated/SettingsView";
import type { ShareView } from "../generated/ShareView";
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
  /** 選べるフォント。フォントのタブを開いたときに取る */
  fonts = $state<FontEntry[]>([]);
  server = $state<Status>("connecting");
  /** 字幕を始められなかった・止まった理由 */
  transcriptionError = $state<TranscriptionError | undefined>();
  /** 直近の入力レベル（dBFS）。字幕を動かしている間だけ届く */
  level = $state<number | undefined>();
  /** OpenAI の使用量と概算の料金 */
  usage = $state<UsageView | undefined>();
  /** 起動してから、字幕を動かして字幕が出た（テスト字幕は数えない） */
  subtitled = $state(false);
  /** もらったギフトコードの状態（保存していれば開いたときに確かめる） */
  gift = $state<GiftStatus | undefined>();
  /** おすそ分け。起動したとき・キーを変えたとき・おすそ分けか料金のタブを開いたときに取り直す */
  share = $state<ShareView | undefined>();
  /** おすそ分けの状態を取れなかった理由 */
  shareError = $state<string | undefined>();
  /** 送った設定の変更の数 */
  private updates = 0;
  /** 最後に SILENT_DB を超えた時刻 */
  private heardAt = 0;

  /** 最後にレベルが届いた時刻 */
  private levelAt = 0;
  now = $state(Date.now());
  /** Chrome のページの状態。ページが開いていなければ undefined */
  chromeStatus = $state<LiveStatus>();

  get settings() {
    return this.view?.settings;
  }

  /** OpenAI の API キーもギフトコードも保存されていない（読めないだけのときは含めない） */
  get keyMissing() {
    return this.view?.keys.openai.state === "missing" && this.view.keys.gift.state === "missing";
  }

  /** 字幕を始められる鍵（自分のキーかギフトコード）がある。Chrome のモードは鍵が要らない */
  get hasAccess() {
    return (
      this.settings?.speechEngine === "chrome" ||
      this.view?.keys.openai.state === "saved" ||
      this.view?.keys.gift.state === "saved"
    );
  }

  /** ハイブリッドで使える鍵（自分のキーか、期限内のギフトコード）がある */
  get hybridReady() {
    return this.view?.keys.openai.state === "saved" || giftValid(this.gift);
  }

  /** 字幕を始める操作をしていて、止まった理由もない */
  get running() {
    return !!this.settings?.transcriptionEnabled && !this.transcriptionError;
  }

  /** 動いているのに、しばらく音が来ていない。Chrome のモードのマイクはページが開く */
  get silent() {
    return (
      this.running &&
      this.settings?.speechEngine !== "chrome" &&
      this.level !== undefined &&
      this.now - this.heardAt > SILENT_MS
    );
  }

  label = (code: string) => languageNames.of(NAME_CODE[code] ?? code) ?? code;

  start() {
    const tick = setInterval(() => (this.now = Date.now()), 500);
    void this.run(async () => {
      [this.view, this.languages] = await Promise.all([api.settings(), api.languages()]);
    });
    void this.refreshDevices();
    void this.refreshGift();
    void this.refreshShare();
    const disconnect = connect(
      ["status", "level", "usage", "subtitle"],
      (msg) => this.onMessage(msg),
      (s) => {
        this.server = s;
        // アプリを更新して開き直したときなど、つなぎ直したら最新の状態（版など）にする
        if (s === "open") void this.refreshSettings();
      },
    );
    return () => {
      clearInterval(tick);
      disconnect();
    };
  }

  private onMessage(msg: ServerMsg) {
    if (msg.type === "transcription.status") {
      this.transcriptionError = msg.error ?? undefined;
    } else if (msg.type === "chrome.status") {
      this.chromeStatus = msg.status ?? undefined;
    } else if (msg.type === "usage") {
      this.usage = msg.usage;
    } else if (msg.type === "settings.changed") {
      void this.refreshSettings();
    } else if (msg.type === "segment.upsert") {
      if (this.running) this.subtitled = true;
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

  /** 続けて変えたとき、古い応答で新しい設定を上書きしない */
  /** 取っている間に設定を変えたら、古い応答で上書きしない */
  refreshSettings = async () => {
    const id = this.updates;
    const view = await api.settings().catch(() => undefined);
    if (view && id === this.updates) this.view = view;
  };

  update = (patch: SettingsPatch) =>
    this.run(async () => {
      const id = ++this.updates;
      const view = await api.updateSettings(patch);
      if (id === this.updates) this.view = view;
    });

  openChrome = () => this.run(api.openChrome);

  resetChrome = () =>
    this.run(async () => {
      await api.resetChrome();
      toast.success("Chrome をリセットしました");
    });

  /** 鍵がないときの入口。押した人にだけ、Chrome のモードに切り替えて始める */
  startWithChrome = () => {
    this.level = undefined;
    return this.update({ speechEngine: "chrome", transcriptionEnabled: true });
  };

  setTranscription(enabled: boolean) {
    this.level = undefined;
    return this.update({ transcriptionEnabled: enabled });
  }

  /** 保存できたら true。預けたキーの差し替えに失敗しても、画面は今の状態にする */
  setKey = async (provider: Provider, key: string) => {
    const saved = await this.run(async () => {
      this.view = await api.setKey(provider, key);
      toast.success(key ? "保存しました" : "削除しました");
    });
    if (!saved) this.view = await api.settings().catch(() => this.view);
    void this.refreshShare();
    return saved;
  };

  loadFonts = async () => {
    if (this.fonts.length > 0) return;
    await this.run(async () => {
      this.fonts = await api.fonts();
    });
  };

  refreshGift = async () => {
    this.gift = await api.gift().catch(() => undefined);
  };

  /** 有効なコードなら保存する。保存できたら true */
  setGift = (code: string) =>
    this.run(async () => {
      const status = await api.setGift(code);
      if (!giftValid(status)) throw new Error(giftStatusText(status));
      this.gift = status;
      this.view = await api.settings();
      toast.success("ギフトコードを保存しました");
    });

  removeGift = () =>
    this.run(async () => {
      await api.removeGift();
      this.gift = { state: "none" };
      this.view = await api.settings();
      toast.success("ギフトコードを削除しました");
    });

  /** 自動で取り直すので、失敗してもトーストは出さない（おすそ分けタブに理由を出す） */
  refreshShare = async () => {
    try {
      this.share = await api.share();
      this.shareError = undefined;
    } catch (e) {
      this.shareError = e instanceof Error ? e.message : String(e);
    }
  };

  /** 失敗したら理由を出す。成功したら true */
  shareAction = (action: () => Promise<ShareView>, done?: string) =>
    this.run(async () => {
      this.share = await action();
      if (done) toast.success(done);
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

/** ギフトコードの 1 日の上限は UTC の日で区切る */
export const LIMIT_RESET = "日本時間の朝 9 時（UTC の 0 時）にまた使えます";

/** 字幕が止まった理由を文にする（サーバーは種類と中身だけを返す） */
export function transcriptionErrorText(e: TranscriptionError): string {
  switch (e.code) {
    case "missingKey":
      return "OpenAI の API キーかギフトコードが設定されていません";
    case "giftExpired":
      return e.expiresAt === null
        ? "ギフトコードの期限が切れました"
        : `ギフトコードの期限が切れました（${dateTime(e.expiresAt)} まで）`;
    case "giftRevoked":
      return "ギフトコードが止められています。おすそ分けしてくれた人に確かめてください";
    case "giftLimit":
      return `ギフトコードの今日の上限に達しました。${LIMIT_RESET}`;
    case "giftUnknown":
      return "ギフトコードが見つかりません。おすそ分けしてくれた人に確かめてください";
    case "invalidKey":
      return "OpenAI の API キーが使えません。キーを確かめてください";
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

/** 有効なコードか。今日の上限は日が変われば戻るので、今は使えなくても有効 */
export const giftValid = (s: GiftStatus | undefined) =>
  s?.state === "active" || s?.state === "limit";

/** ギフトコードの状態を文にする（使えるときは期限） */
export function giftStatusText(s: GiftStatus): string {
  switch (s.state) {
    case "none":
      return "ギフトコードがありません";
    case "active":
      return s.expiresAt === null ? "期限なしで使えます" : `${dateTime(s.expiresAt)} まで使えます`;
    case "expired":
      return `期限が切れています（${dateTime(s.expiresAt)} まで）`;
    case "revoked":
      return "おすそ分けした人が止めています";
    case "limit":
      return `今日の上限に達しました。${LIMIT_RESET}`;
    case "unknown":
      return "このコードは見つかりません。打ち間違いがないか確かめてください";
    case "unreachable":
      return `サーバーにつながりません（${s.detail}）`;
    default:
      return unknown(s);
  }
}

/** 「11/10（月）12:00」 */
export function dateTime(ms: number): string {
  return new Intl.DateTimeFormat("ja", {
    month: "numeric",
    day: "numeric",
    weekday: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(ms);
}
