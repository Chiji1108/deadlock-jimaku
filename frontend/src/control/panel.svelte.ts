// 操作パネルの状態。各タブはこれを読み、変更は api 経由でサーバーに送る（サーバーの返す設定で置き換える）
import { toast } from "svelte-sonner";
import { SvelteMap, SvelteSet } from "svelte/reactivity";
import type { LiveStatus } from "../generated/LiveStatus";
import type { BetaRefusal } from "../generated/BetaRefusal";
import type { Engines } from "../generated/Engines";
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
import {
  CHROME,
  PROVIDER_NAMES,
  chromeFinals,
  giftable,
  keyNames,
  providers,
  relayOf,
} from "../lib/engines";
import { addFontFaces, fontWeights } from "../lib/overlay-fonts";
import type { MergePatch } from "../lib/text-style";
import type { FontEntry } from "../generated/FontEntry";
import type { FontStyle } from "../generated/FontStyle";
import { connect, type Status } from "../lib/ws";

/** 選べるフォントの一覧。取れるまでと、取れなかったときは名前の配列の代わりにその状態が入る */
export type FontList = FontEntry[] | "loading" | "failed";
/** フォントを選ぶ先。字幕は言語ごと、ランクは 1 つ */
export type FontTarget = { kind: "subtitle"; language: string } | { kind: "rank" };

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
  /** 選べるフォント */
  fonts = $state<FontList>("loading");
  /** ダウンロード中のファミリー。タブを移っても続くので、ここで持つ */
  downloadingFonts = new SvelteSet<string>();
  private fontsRead: Promise<void> | undefined;
  /** 見本の SVG。読み終えるまでは空で、カードは名前だけで押せる */
  fontSamples = $state.raw<Record<string, string>>({});
  private samplesRead: Promise<void> | undefined;
  server = $state<Status>("connecting");
  /** 字幕を始められなかった・止まった理由 */
  /** 開いているタブ（ほかのタブへの案内から切り替える） */
  tab = $state("audio");
  transcriptionError = $state<TranscriptionError | undefined>();
  /** PC の中の文字起こしを準備している（初回はダウンロードを待つ） */
  preparing = $state(false);
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

  /** 選んだ作り方に要る会社のうち、自分のキーがないもの */
  private get withoutKey(): Provider[] {
    const view = this.view;
    if (!view) return [];
    return providers(view.settings.engines).filter((p) => view.keys[p].state !== "saved");
  }

  /** 選んだ作り方に要るのに、自分のキーもギフトコードもない会社。ギフトコードが xAI を中継するかは、
   * 字幕を始めるときに中核が確かめる（中継しなければ `giftUnsupported` で止まる） */
  get missingKeys(): Provider[] {
    const gift = this.view?.keys.gift.state === "saved";
    return this.withoutKey.filter((p) => !gift || !giftable(p));
  }

  /** ギフトコードで使う（おすそ分けしてくれた人に請求される）会社。コードが中継する会社が分かれば、それだけ */
  get giftPaid(): Provider[] {
    if (this.view?.keys.gift.state !== "saved") return [];
    const relayed = this.gift?.state === "active" ? this.gift.providers : undefined;
    return this.withoutKey.filter((p) => giftable(p) && (!relayed || relayed.includes(p)));
  }

  /** 字幕を始められる鍵がそろっている。有料の工程がなければ鍵は要らない */
  get hasAccess() {
    return !!this.view && this.missingKeys.length === 0;
  }

  /** 選んだモードに要る鍵がない。キーの案内を出し、字幕を始めさせない */
  get needsKey() {
    return this.missingKeys.length > 0;
  }

  /** 字幕を始める操作をしていて、止まった理由もない */
  get running() {
    return !!this.settings?.transcriptionEnabled && !this.transcriptionError;
  }

  /** 動いているのに、しばらく音が来ていない。確定の文を Chrome が作るなら、マイクはページが開く */
  get silent() {
    return (
      this.running &&
      !!this.settings &&
      !chromeFinals(this.settings.engines) &&
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
    // WebView が名前で描ける PC のフォントは、起動したときにあったもの。その一覧をここで覚える
    void this.loadFonts();
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
      this.preparing = msg.preparing;
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
    return this.update({ engines: CHROME, transcriptionEnabled: true });
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

  private knownFonts: Set<string> | undefined;

  /** 同時には 1 回だけ読む。読んでいる間に呼んだら、その読み込みの終わりを待つ */
  loadFonts = () =>
    (this.fontsRead ??= this.readFonts().finally(() => (this.fontsRead = undefined)));

  /** 読み直しの間も、失敗しても、すでにある一覧は出したままにする */
  private async readFonts() {
    const fonts = await api.fonts().catch(() => undefined);
    if (fonts) {
      this.addFaces(fonts);
      this.fonts = fonts;
    } else if (typeof this.fonts === "string") this.fonts = "failed";
  }

  /**
   * 最初の一覧（`start` で読む）の PC のフォントは WebView が名前で描ける。そのあと増えたフォントと、
   * PC になくダウンロードで置いたフォントは読み込む。未取得のフォントはファイルがないので読まない
   */
  private addFaces(fonts: FontEntry[]) {
    const first = !this.knownFonts;
    const known = (this.knownFonts ??= new Set());
    for (const { family, sizeBytes, downloaded } of fonts) {
      if (sizeBytes !== undefined || known.has(family)) continue;
      known.add(family);
      if (!first || downloaded) addFontFaces(family);
    }
  }

  /** 読めなければ次に開いたときに読み直す */
  loadFontSamples = () =>
    (this.samplesRead ??= import("./font-sample-svgs").then(
      ({ SAMPLES }) => void (this.fontSamples = SAMPLES),
      () => void (this.samplesRead = undefined),
    ));

  /** フォントの持つ太さ。読み終えるまでは undefined */
  weights = new SvelteMap<string, number[]>();

  private readingWeights = new Set<string>();

  /** 読めなければ覚えず、次に開いたときに読み直す */
  loadWeights = async (family: string) => {
    if (this.weights.has(family) || this.readingWeights.has(family)) return;
    this.readingWeights.add(family);
    const weights = await fontWeights(family).catch(() => undefined);
    this.readingWeights.delete(family);
    if (weights) this.weights.set(family, weights);
  };

  fontStyle = (target: FontTarget): FontStyle | undefined =>
    target.kind === "rank"
      ? this.settings?.rankStyle
      : this.settings?.subtitleStyle.languages[target.language];

  chosenFont = (target: FontTarget) => this.fontStyle(target)?.font ?? undefined;

  setFontStyle = (target: FontTarget, patch: MergePatch<FontStyle>) =>
    this.update(
      target.kind === "rank"
        ? { rankStyle: patch }
        : { subtitleStyle: { languages: { [target.language]: patch } } },
    );

  /** 空文字列なら標準に戻す */
  setFont = (target: FontTarget, family: string) =>
    this.setFontStyle(target, { font: family || null });

  /**
   * 裏で進め、取得できたら一覧を読み直して選ぶ先に適用する。走行中の読み込みは取得前の一覧を
   * 返すので、終わるのを待ってから読む。待つ間に別のフォントを選んでいたら、その選択を残す
   */
  downloadFont = async (family: string, target: FontTarget) => {
    const chosen = this.chosenFont(target);
    const progress = toast.loading(`${family} をダウンロードしています`, {
      duration: Number.POSITIVE_INFINITY,
    });
    this.downloadingFonts.add(family);
    const done = await this.run(async () => {
      await api.downloadFont(family);
      await this.fontsRead;
      await this.loadFonts();
    });
    this.downloadingFonts.delete(family);
    toast.dismiss(progress);
    if (!done) return;
    toast.success(`${family} をダウンロードしました`);
    if (this.chosenFont(target) === chosen) await this.setFont(target, family);
  };

  refreshGift = async () => {
    this.gift = await api.gift().catch(() => undefined);
  };

  /** モードを選ぶ。ベータのモードは、コードがなければ自動でもらう（DESIGN.md 17.2） */
  chooseMode = async (engines: Engines) => {
    await this.update({ engines });
    const beta = providers(engines).some((p) => relayOf(p) === "beta");
    if (!beta || this.missingKeys.length === 0 || this.view?.keys.gift.state === "saved") return;
    await this.run(async () => {
      const joined = await api.joinBeta();
      if ("error" in joined) throw new Error(betaRefusalText(joined));
      this.gift = joined;
      this.view = await api.settings();
      toast.success("ベータを使えるようにしました");
    });
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
      return `${keyNames([e.provider])}が設定されていません`;
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
    case "giftUnsupported":
      return relayOf(e.provider) === "beta"
        ? `持っているコードでは ${PROVIDER_NAMES[e.provider]} の この工程を使えません。ベータのモードにするか、自分の API キーを入れてください（人からもらったギフトコードがあると、ベータのコードはもらえません）`
        : `このギフトコードでは ${PROVIDER_NAMES[e.provider]} を使えません。おすそ分けしてくれた人に ${PROVIDER_NAMES[e.provider]} のキーも預けてもらうか、自分の API キーを入れてください`;
    case "invalidKey":
      return `${PROVIDER_NAMES[e.provider]} の API キーが使えません。キーを確かめてください`;
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
/** ベータのコードをもらえなかった理由を文にする */
export function betaRefusalText(e: BetaRefusal): string {
  switch (e.error) {
    case "closed":
      return "ベータの無料の提供は終わりました。ほかのモードを選んでください";
    case "tooMany":
      return "今日はもう、ベータのコードを何度も受け取っています。明日また試してください";
    case "full":
      return "今日のベータの受け付けは終わりました。明日また試してください";
    case "unreachable":
      return `ベータのサーバーにつながりません（${e.detail}）`;
    default:
      return unknown(e);
  }
}

export const giftValid = (s: GiftStatus | undefined) =>
  s?.state === "active" || s?.state === "limit";

/** ギフトコードの状態を文にする（使えるときは期限） */
export function giftStatusText(s: GiftStatus): string {
  switch (s.state) {
    case "none":
      return "ギフトコードがありません";
    case "active": {
      const until = s.expiresAt === null ? "期限なしで" : `${dateTime(s.expiresAt)} まで`;
      const names = s.providers.map((p) => PROVIDER_NAMES[p]).join(" と ");
      return `${until}使えます（${names}）`;
    }
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
