// 字幕の作り方。モードで工程ごとの作り方がまとめて決まり、詳しく選ぶと工程ごとに変えられる。
// 操作パネルのモードの欄と LP のモードの節が、同じ並びと言い方で出す
import type { Engines } from "../generated/Engines";
import type { Provider } from "../generated/Provider";

type Item = keyof Engines;

export type Option<K extends Item = Item> = {
  value: Engines[K];
  label: string;
  /** 料金（`crates/core/src/usage.rs` の単価と同じ）。無料なら「無料」 */
  price: string;
  /** 有料なら、使う会社（その鍵が要る）。無料なら undefined */
  provider?: Provider;
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
        provider: "openai",
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
        provider: "openai",
      },
      {
        value: "grokVoiceTranscribe",
        label: "Grok Voice Transcribe",
        price: "話した 1 分 $0.0017",
        provider: "xai",
        note: "xAI の API キーが要る。GPT-Transcribe の約 3 分の 1 の料金で、精度は同じか少し下",
      },
    ],
  },
  translation: {
    label: "翻訳と用語",
    hint: "訳文と、ヒーロー・スキル・アイテムの判定。訳文の名前を用語から作るので、いっしょに選びます",
    options: [
      { value: "chromeTranslator", label: "Chrome の翻訳 ＋ 辞書", price: "無料" },
      {
        value: "gpt61Terra",
        label: "GPT-6.1 Terra",
        price: "100 万トークンで入力 $1・出力 $5",
        provider: "openai",
        note: "略称やスキルの呼び方もよく判定する",
      },
      {
        value: "gpt6Luna",
        label: "GPT-6 Luna",
        price: "100 万トークンで入力 $0.1・出力 $0.5",
        provider: "openai",
        note: "Terra の約 6 分の 1 の料金。「〇〇のウルト」などの判定は Terra より弱い",
      },
    ],
  },
  furigana: {
    label: "ふりがな",
    hint: "日本語の漢字の読み",
    options: [
      { value: "lindera", label: "辞書（PC の中）", price: "無料" },
      {
        value: "llm",
        label: "GPT-6.1 Terra",
        price: "100 万トークンで入力 $1・出力 $5",
        provider: "openai",
      },
    ],
  },
};

/** モード。`crates/core/src/settings.rs` の `Engines` の定数と同じ */
export const CHROME: Engines = {
  live: "chrome",
  transcription: "chrome",
  translation: "chromeTranslator",
  furigana: "lindera",
};
export const HYBRID: Engines = {
  live: "chrome",
  transcription: "gptTranscribe",
  translation: "gpt61Terra",
  furigana: "llm",
};
const OPENAI: Engines = { ...HYBRID, live: "gptLiveTranscribe" };

export const MODES: {
  name: string;
  label: string;
  traits: string;
  price: string;
  hint: string;
  engines: Engines;
  recommended?: boolean;
}[] = [
  {
    name: "chrome",
    label: "Chrome",
    traits: "高速・キー不要",
    price: "無料",
    hint: "多くの字幕ソフトと同じく、Chrome の音声認識を使います",
    engines: CHROME,
  },
  {
    name: "hybrid",
    label: "ハイブリッド",
    traits: "高速・高精度",
    price: "約 $0.6／時間",
    hint: "途中の字幕は Chrome、確定の字幕は OpenAI の高い精度に置き換えます",
    engines: HYBRID,
    recommended: true,
  },
  {
    name: "openai",
    label: "OpenAI",
    traits: "低負荷・高精度",
    price: "約 $1／時間",
    hint: "すべて OpenAI で作ります。Chrome は使いません",
    engines: OPENAI,
  },
];

const same = (a: Engines, b: Engines) =>
  (Object.keys(ITEMS) as Item[]).every((item) => a[item] === b[item]);

/** 今の選び方に合うモード。詳しく選んで変えたなら undefined */
export const modeOf = (t: Engines) => MODES.find((m) => same(m.engines, t));

/** Chrome のウィンドウを使う（途中の字幕が Chrome） */
export const usesChrome = (t: Engines) => t.live === "chrome";

/** 発話の区切りを Chrome が決める（アプリはマイクを開かない） */
export const chromeFinals = (t: Engines) => t.transcription === "chrome";

const providerOf = (t: Engines, item: Item) =>
  ITEMS[item].options.find((o) => o.value === t[item])?.provider;

/** 有料の工程で使う会社（その鍵が要る）。OpenAI を先に置く */
export const providers = (t: Engines): Provider[] =>
  (["openai", "xai"] as const).filter((p) =>
    (Object.keys(ITEMS) as Item[]).some((item) => providerOf(t, item) === p),
  );

/** 有料の工程がある（鍵が要る） */
export const anyPaid = (t: Engines) => providers(t).length > 0;

/** 会社の名前 */
export const PROVIDER_NAMES: Record<Provider, string> = { openai: "OpenAI", xai: "xAI" };

/** 会社ごとの、要る鍵の名前。ギフトコードは OpenAI にだけ使える */
export const KEY_NAMES: Record<Provider, string> = {
  openai: "OpenAI の API キーかギフトコード",
  xai: "xAI の API キー",
};

/** 要る鍵の名前をつなげる */
export const keyNames = (list: Provider[]) => list.map((p) => KEY_NAMES[p]).join("と ");

/** 今の仕組みで作れない選び方。core の `Engines::sanitized` と同じ決まり。
 * Chrome の翻訳を使っている間は途中の字幕を Chrome から変えさせない（変えると core が翻訳を有料に直し、黙って料金がかかる） */
export function unavailable<K extends Item>(t: Engines, item: K, value: Engines[K]) {
  if (item === "live")
    return (chromeFinals(t) || t.translation === "chromeTranslator") && value !== "chrome";
  if (item === "translation") return !usesChrome(t) && value === "chromeTranslator";
  return false;
}

/** 有料の工程の名前（詳しく選んだときの説明に使う） */
export const paidItems = (t: Engines) =>
  (Object.keys(ITEMS) as Item[])
    .filter((item) => providerOf(t, item))
    .map((item) => ITEMS[item].label);
