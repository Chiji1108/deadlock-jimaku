// 字幕の作り方。モードで工程ごとの作り方がまとめて決まり、詳しく選ぶと工程ごとに変えられる。
// モードは `crates/core/src/settings.rs` の `Engines` の定数と同じ組み合わせ
// 操作パネルのモードの欄と LP のモードの節が、同じ並びと言い方で出す
import { ENGINE_PROVIDERS, PROVIDERS } from "../generated/catalog";
import type { Engines } from "../generated/Engines";
import type { Provider } from "../generated/Provider";

type Item = keyof Engines;

export type Option<K extends Item = Item> = {
  value: Engines[K];
  label: string;
  /** 料金（`crates/core/src/usage.rs` の単価と同じ）。無料なら「無料」。
   * 使う会社は `ENGINE_PROVIDERS`（core が決める）にあるので、ここには書かない */
  price: string;
  /** ほかの選択肢との違い（測った結果。DESIGN.md 9.3） */
  note?: string;
};

/** 工程と、その作り方。無料のものを先に置く */
export const ITEMS: { [K in Item]: { label: string; hint: string; options: Option<K>[] } } = {
  live: {
    label: "途中の字幕",
    hint: "話している間に出す字幕",
    options: [
      { value: "chrome", label: "Chrome", price: "無料" },
      {
        value: "gptLiveTranscribe",
        label: "GPT-Live-Transcribe",
        price: "話した 1 分 $0.017",
      },
      {
        value: "qwenAudioStreaming",
        label: "Qwen-Audio-3.1-ASR-Flash-Streaming",
        price: "話した 1 分 約 $0.0008",
        note: "Chrome を使わない。GPT-Live-Transcribe の約 20 分の 1 の料金で、精度は同じくらい。用語のヒントは渡せない",
      },
      {
        value: "haasJa",
        label: "Haas-ja（PC の中）",
        price: "無料",
        note: "日本語だけ。Chrome を使わず、音声は PC から出ない。話している間、PC の中で何度も認識し直すので、PC の負荷は上がる。初回に約 400 MB をダウンロードする",
      },
      { value: "off", label: "出さない", price: "無料" },
    ],
  },
  transcription: {
    label: "確定の文字起こし",
    hint: "話し終えたあとに置き換える字幕",
    options: [
      { value: "chrome", label: "Chrome", price: "無料" },
      {
        value: "gptTranscribe",
        label: "GPT-Transcribe",
        price: "話した 1 分 $0.0045",
      },
      {
        value: "grokVoiceTranscribe",
        label: "Grok Voice Transcribe",
        price: "話した 1 分 $0.0017",
        note: "GPT-Transcribe の約 3 分の 1 の料金で、精度は同じか少し下",
      },
      {
        value: "qwenAudio",
        label: "Qwen-Audio-3.1-ASR-Flash",
        price: "話した 1 分 約 $0.0002",
        note: "Grok の約 7 分の 1 の料金で、精度は同じくらい。Grok より 1 秒ほど遅い",
      },
      {
        value: "maiTranscribe2",
        label: "MAI-Transcribe 2",
        price: "話した 1 分 $0.0017",
        note: "精度も料金も Grok と同じくらい。用語のヒントは渡せない",
      },
      {
        value: "haasJa",
        label: "Haas-ja（PC の中）",
        price: "無料",
        note: "日本語だけ。音声は PC から出ない。用語を教えられないので、カタカナの用語は Grok や Qwen より間違えやすい。初回に約 400 MB をダウンロードする。Intel の Mac では動かない",
      },
    ],
  },
  terms: {
    label: "用語",
    hint: "ヒーロー・スキル・アイテムの判定。翻訳を GPT にしたときは、翻訳と同時に判定するので選びません",
    options: [
      {
        value: "dictionary",
        label: "辞書の一致（PC の中）",
        price: "無料",
        note: "正式な名前だけ",
      },
      {
        value: "geminiNano",
        label: "Gemini Nano（Chrome の中で確かめる）",
        price: "無料",
        note: "略称や「〇〇のウルト」も判定する。文字を PC から出さない。会社の判定モデルほどは当たらない。初回に Chrome が約 4 GB をダウンロードし、GPU を使う。使えない PC では辞書の一致になる",
      },
      {
        value: "decisions",
        label: "GPT-6 Luna（確かめるだけ）",
        price: "100 万トークンで入力 $0.1",
        note: "略称・聞き違い・「〇〇のウルト」も判定する。PC の中で出した候補を確かめるだけなので、速くて安い",
      },
      {
        value: "jev",
        label: "Jev（確かめるだけ）",
        price: "GPT-6 Luna の約 6 割",
        note: "いちばん速い。精度は GPT-6 Luna と同じくらい",
      },
      {
        value: "drexMax",
        label: "Drex Max（確かめるだけ）",
        price: "GPT-6 Luna の約半分",
        note: "いちばんよく当たった。GPT-6 Luna より 0.2 秒ほど遅い",
      },
      {
        value: "decision1",
        label: "Microsoft Decision 1（確かめるだけ）",
        price: "GPT-6 Luna の約 3 割",
        note: "GPT-6 Luna より少しよく当たり、0.2 秒ほど遅い",
      },
      {
        value: "pplxDecider",
        label: "Perplexity Decider（確かめるだけ）",
        price: "GPT-6 Luna の約 6 分の 1",
        note: "いちばん安い。GPT-6 Luna より少しよく当たり、0.2 秒ほど遅い",
      },
    ],
  },
  translation: {
    label: "翻訳",
    hint: "訳文。GPT は用語も同時に判定します（訳文の名前を用語から作るので、1 回で聞きます）",
    options: [
      { value: "chromeTranslator", label: "Chrome の翻訳", price: "無料" },
      {
        value: "qwenMtFlash",
        label: "Qwen-MT-Flash",
        price: "100 万トークンで入力 $0.16・出力 $0.49",
        note: "翻訳専門のモデル。速くて安い",
      },
      {
        value: "hyMt2",
        label: "Hy-MT2",
        price: "100 万トークンで入力 $0.074・出力 $0.295",
        note: "翻訳専門のモデル。Qwen-MT-Flash の約半分の料金で、0.5 秒ほど遅い",
      },
      {
        value: "gpt61Terra",
        label: "GPT-6.1 Terra（用語も）",
        price: "100 万トークンで入力 $1・出力 $5",
        note: "訳がいちばん自然で、略称やスキルの呼び方もよく判定する",
      },
    ],
  },
  furigana: {
    label: "ふりがな",
    hint: "日本語の漢字の読み",
    options: [
      {
        value: "dictionary",
        label: "辞書（PC の中）",
        price: "無料",
        note: "話し言葉向けの辞書で読む。文の意味で読みが変わる語（辛い、人気）は間違えることがある",
      },
      {
        value: "decider",
        label: "辞書 + Perplexity Decider",
        price: "ほぼ無料（1 時間で 1 セント未満）",
        note: "読みが分かれる語を、辞書の候補から選ぶ。速い。まれに、合っていた読みを替えてしまう",
      },
      {
        value: "llm",
        label: "辞書 + GPT-6.1 Terra",
        price: "100 万トークンで入力 $1・出力 $5",
        note: "辞書の読みを、文に合わせて直す。いちばん正確",
      },
      {
        value: "luna",
        label: "辞書 + GPT-6 Luna",
        price: "Terra の約 7 分の 1",
        note: "辞書の読みを、文に合わせて直す。Terra より少し直し残す",
      },
    ],
  },
};

/** モードの組み合わせ */
export const CHROME: Engines = {
  live: "chrome",
  transcription: "chrome",
  terms: "dictionary",
  translation: "chromeTranslator",
  furigana: "dictionary",
};
/** ベータの間は無料の組み合わせ（開発者のサーバーが中継する。DESIGN.md 17.2） */
export const BETA: Engines = {
  live: "chrome",
  transcription: "qwenAudio",
  terms: "pplxDecider",
  translation: "hyMt2",
  furigana: "decider",
};

/** 最高精度の組み合わせ（0.5 までの「スタンダード」）。確定の文字起こし・翻訳と用語・ふりがなを GPT で作る */
export const BEST: Engines = {
  live: "chrome",
  transcription: "gptTranscribe",
  terms: "dictionary",
  translation: "gpt61Terra",
  furigana: "llm",
};

/** 月の目安の使い方（毎日 3 時間、30 日） */
const MONTHLY_HOURS = 3 * 30;
/** 月の目安を円にするときの 1 ドル（為替で動くので目安） */
export const YEN_PER_USD = 150;

/** 月の目安。「毎日 3 時間で月 約 ¥8,100（$54）」 */
export function monthly(hourlyUsd: number) {
  const usd = hourlyUsd * MONTHLY_HOURS;
  const yen = Math.round((usd * YEN_PER_USD) / 100) * 100;
  return `毎日 3 時間で月 約 ¥${yen.toLocaleString("ja-JP")}（$${Math.round(usd)}）`;
}

/** おすすめの組み合わせ。`crates/core/src/settings.rs` の `Engines::MODES` と同じ。ほかの組み合わせは「カスタム」 */
export const MODES: {
  name: string;
  label: string;
  traits: string;
  /** 1 時間の目安（DESIGN.md 3.3）。無料なら 0 */
  hourlyUsd: number;
  hint: string;
  engines: Engines;
  /** 名前の横に出すひとこと */
  badge?: string;
}[] = [
  {
    name: "beta",
    label: "おすすめ",
    traits: "高精度・キー不要",
    hourlyUsd: 0,
    hint: "ベータの間は無料です。途中の字幕は Chrome、確定の字幕・用語・翻訳・ふりがなの読み分けは、開発者のサーバーを通して作ります（1 日の上限があります）",
    engines: BETA,
    badge: "ベータの間は無料",
  },
  {
    name: "chrome",
    label: "Chrome",
    traits: "サーバーを使わない・キー不要",
    hourlyUsd: 0,
    hint: "多くの字幕ソフトと同じく、Chrome の音声認識と翻訳だけを使います。開発者のサーバーは使いません",
    engines: CHROME,
  },
  {
    name: "best",
    label: "最高精度",
    traits: "OpenAI のキーが要る",
    hourlyUsd: 0.6,
    hint: "途中の字幕は Chrome、確定の字幕・翻訳・ふりがなは OpenAI の GPT で作ります。用語の判定と文字起こしがいちばん正確ですが、確定の字幕が出るまで 1〜2 秒長くかかります",
    engines: BEST,
  },
];

/** 翻訳が GPT なら、用語も GPT が同時に判定する（用語の選択は使わない） */
export const llmTranslation = (t: Engines) => t.translation === "gpt61Terra";

/** 実際に使う用語の作り方 */
const effective = (t: Engines): Engines => (llmTranslation(t) ? { ...t, terms: "dictionary" } : t);

const same = (a: Engines, b: Engines) =>
  (Object.keys(ITEMS) as Item[]).every((item) => effective(a)[item] === effective(b)[item]);

/** 今の選び方に合うモード。詳しく選んで変えたなら undefined */
export const modeOf = (t: Engines) => MODES.find((m) => same(m.engines, t));

/** 途中の字幕をアプリが作る（有料の音声認識につなぐ） */
export const paidLive = (t: Engines) => providerOfOption("live", t.live) !== null;

/** Chrome のウィンドウを使う（途中の字幕が Chrome） */
export const usesChrome = (t: Engines) => t.live === "chrome";

/** 発話の区切りを Chrome が決める（アプリはマイクを開かない） */
export const chromeFinals = (t: Engines) => t.transcription === "chrome";

/** その選択肢が使う会社（その鍵が要る）。無料なら null */
export const providerOfOption = <K extends Item>(item: K, value: Engines[K]): Provider | null =>
  (ENGINE_PROVIDERS[item] as Record<string, Provider | null>)[value] ?? null;

const providerOf = (t: Engines, item: Item) => providerOfOption(item, effective(t)[item]);

/** 有料の工程で使う会社（その鍵が要る）。OpenAI を先に置く */
export const providers = (t: Engines): Provider[] =>
  PROVIDERS.map((p) => p.id).filter((p) =>
    (Object.keys(ITEMS) as Item[]).some((item) => providerOf(t, item) === p),
  );

/** 有料の工程がある（鍵が要る） */
export const anyPaid = (t: Engines) => providers(t).length > 0;

/** 会社の名前 */
export const PROVIDER_NAMES = Object.fromEntries(PROVIDERS.map((p) => [p.id, p.name])) as Record<
  Provider,
  string
>;

/** その会社を中継するコードの種類。gift は人からもらったギフトコード（おすそ分けした人がその会社のキーを預けていれば）、
 * beta はベータのコード（開発者のキー）。中継がなければ null */
export const relayOf = (p: Provider) => PROVIDERS.find((info) => info.id === p)?.relay ?? null;

/** コード（ギフトコードかベータのコード）で使える会社。どちらも同じ所に保存する */
export const giftable = (p: Provider) => relayOf(p) !== null;

/** 要る鍵の名前。ギフトコードで足りる会社があれば、そう添える */
export const keyNames = (list: Provider[]) =>
  `${list.map((p) => PROVIDER_NAMES[p]).join(" と ")} の API キー${list.some((p) => relayOf(p) === "gift") ? "かギフトコード" : ""}`;

/** 今の仕組みで作れない選び方。core の `Engines::sanitized` と同じ決まり。
 * Chrome の翻訳を使っている間は途中の字幕を Chrome から変えさせない（変えると core が翻訳を有料に直し、黙って料金がかかる） */
export function unavailable<K extends Item>(t: Engines, item: K, value: Engines[K]) {
  if (item === "live")
    return (chromeFinals(t) || t.translation === "chromeTranslator") && value !== "chrome";
  if (item === "translation") return !usesChrome(t) && value === "chromeTranslator";
  // Gemini Nano は Chrome のページの中にある
  if (item === "terms") return llmTranslation(t) || (value === "geminiNano" && !usesChrome(t));
  return false;
}

/** 有料の工程の名前（詳しく選んだときの説明に使う） */
export const paidItems = (t: Engines) =>
  (Object.keys(ITEMS) as Item[])
    .filter((item) => providerOf(t, item))
    .map((item) => ITEMS[item].label);

/** モードの値段。「無料」か「約 $0.6／時間」 */
export const price = (hourlyUsd: number) => (hourlyUsd ? `約 $${hourlyUsd}／時間` : "無料");
