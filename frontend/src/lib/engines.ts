// 字幕の作り方。モードで工程ごとの作り方がまとめて決まり、詳しく選ぶと工程ごとに変えられる。
// モードは `crates/core/src/settings.rs` の `Engines` の定数と同じ組み合わせ
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
        value: "decisions",
        label: "GPT-6 Luna（確かめるだけ）",
        price: "100 万トークンで入力 $0.1",
        provider: "openai",
        note: "略称・聞き違い・「〇〇のウルト」も判定する。PC の中で出した候補を確かめるだけなので、速くて安い",
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
        provider: "qwen",
        note: "翻訳専門のモデル。速くて安い。Qwen の API キーが要る（ギフトコードでは使えない）",
      },
      {
        value: "gpt61Terra",
        label: "GPT-6.1 Terra（用語も）",
        price: "100 万トークンで入力 $1・出力 $5",
        provider: "openai",
        note: "訳がいちばん自然で、略称やスキルの呼び方もよく判定する",
      },
      {
        value: "gpt6Luna",
        label: "GPT-6 Luna（用語も）",
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

/** モードの組み合わせ */
export const CHROME: Engines = {
  live: "chrome",
  transcription: "chrome",
  terms: "dictionary",
  translation: "chromeTranslator",
  furigana: "lindera",
};
export const STANDARD: Engines = {
  live: "chrome",
  transcription: "gptTranscribe",
  terms: "dictionary",
  translation: "gpt61Terra",
  furigana: "llm",
};
const OPENAI: Engines = { ...STANDARD, live: "gptLiveTranscribe" };
const LIGHT: Engines = {
  live: "chrome",
  transcription: "grokVoiceTranscribe",
  terms: "dictionary",
  translation: "gpt6Luna",
  furigana: "lindera",
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

export const MODES: {
  name: string;
  label: string;
  traits: string;
  /** 1 時間の目安（DESIGN.md 3.3）。無料なら 0 */
  hourlyUsd: number;
  hint: string;
  engines: Engines;
  recommended?: boolean;
}[] = [
  {
    name: "chrome",
    label: "Chrome",
    traits: "高速・キー不要",
    hourlyUsd: 0,
    hint: "多くの字幕ソフトと同じく、Chrome の音声認識を使います",
    engines: CHROME,
  },
  {
    name: "light",
    label: "ライト",
    traits: "高速・高精度",
    hourlyUsd: 0.1,
    hint: "途中の字幕は Chrome、確定の字幕は xAI の Grok、翻訳は OpenAI の安いモデルで作ります",
    engines: LIGHT,
  },
  {
    name: "standard",
    label: "スタンダード",
    traits: "高速・最高精度",
    hourlyUsd: 0.6,
    hint: "途中の字幕は Chrome、確定の字幕は OpenAI の高い精度に置き換えます",
    engines: STANDARD,
    recommended: true,
  },
  {
    name: "openai",
    label: "OpenAI",
    traits: "低負荷・最高精度",
    hourlyUsd: 1,
    hint: "すべて OpenAI で作ります。Chrome は使いません",
    engines: OPENAI,
  },
];

/** 翻訳が GPT なら、用語も GPT が同時に判定する（用語の選択は使わない） */
export const llmTranslation = (t: Engines) =>
  t.translation === "gpt61Terra" || t.translation === "gpt6Luna";

/** 実際に使う用語の作り方 */
const effective = (t: Engines): Engines => (llmTranslation(t) ? { ...t, terms: "dictionary" } : t);

const same = (a: Engines, b: Engines) =>
  (Object.keys(ITEMS) as Item[]).every((item) => effective(a)[item] === effective(b)[item]);

/** 今の選び方に合うモード。詳しく選んで変えたなら undefined */
export const modeOf = (t: Engines) => MODES.find((m) => same(m.engines, t));

/** Chrome のウィンドウを使う（途中の字幕が Chrome） */
export const usesChrome = (t: Engines) => t.live === "chrome";

/** 発話の区切りを Chrome が決める（アプリはマイクを開かない） */
export const chromeFinals = (t: Engines) => t.transcription === "chrome";

const providerOf = (t: Engines, item: Item) =>
  ITEMS[item].options.find((o) => o.value === effective(t)[item])?.provider;

/** 有料の工程で使う会社（その鍵が要る）。OpenAI を先に置く */
export const providers = (t: Engines): Provider[] =>
  (["openai", "xai", "qwen"] as const).filter((p) =>
    (Object.keys(ITEMS) as Item[]).some((item) => providerOf(t, item) === p),
  );

/** 有料の工程がある（鍵が要る） */
export const anyPaid = (t: Engines) => providers(t).length > 0;

/** 会社の名前 */
export const PROVIDER_NAMES: Record<Provider, string> = {
  openai: "OpenAI",
  xai: "xAI",
  qwen: "Qwen",
};

/** ギフトコードで使える会社（おすそ分けした人がその会社のキーを預けていれば）。Qwen の中継はまだない */
export const giftable = (p: Provider) => p !== "qwen";

/** 要る鍵の名前。ギフトコードで足りる会社があれば、そう添える */
export const keyNames = (list: Provider[]) =>
  `${list.map((p) => PROVIDER_NAMES[p]).join(" と ")} の API キー${list.some(giftable) ? "かギフトコード" : ""}`;

/** 今の仕組みで作れない選び方。core の `Engines::sanitized` と同じ決まり。
 * Chrome の翻訳を使っている間は途中の字幕を Chrome から変えさせない（変えると core が翻訳を有料に直し、黙って料金がかかる） */
export function unavailable<K extends Item>(t: Engines, item: K, value: Engines[K]) {
  if (item === "live")
    return (chromeFinals(t) || t.translation === "chromeTranslator") && value !== "chrome";
  if (item === "translation") return !usesChrome(t) && value === "chromeTranslator";
  if (item === "terms") return llmTranslation(t);
  return false;
}

/** 有料の工程の名前（詳しく選んだときの説明に使う） */
export const paidItems = (t: Engines) =>
  (Object.keys(ITEMS) as Item[])
    .filter((item) => providerOf(t, item))
    .map((item) => ITEMS[item].label);

/** モードの値段。「無料」か「約 $0.6／時間」 */
export const price = (hourlyUsd: number) => (hourlyUsd ? `約 $${hourlyUsd}／時間` : "無料");
