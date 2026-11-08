// 字幕の作り方。品ごとに無料か有料かを選ぶ。操作パネルの「字幕の作り方」の欄と LP の節が、同じ並びと言い方で出す
import type { Toppings } from "../generated/Toppings";

type Item = keyof Toppings;

export type Option<K extends Item = Item> = {
  value: Toppings[K];
  label: string;
  /** 料金（`crates/core/src/usage.rs` の単価と同じ）。無料なら「無料」 */
  price: string;
  /** OpenAI の API キーかギフトコードが要る */
  paid: boolean;
};

/** 品と、その選び方。無料のものを先に置く */
export const ITEMS: { [K in Item]: { label: string; hint: string; options: Option<K>[] } } = {
  live: {
    label: "途中の字幕",
    hint: "話している間に出す字幕",
    options: [
      { value: "chrome", label: "Chrome", price: "無料", paid: false },
      {
        value: "gptLiveTranscribe",
        label: "GPT-Live-Transcribe",
        price: "話した 1 分 $0.017",
        paid: true,
      },
      { value: "off", label: "出さない", price: "無料", paid: false },
    ],
  },
  transcription: {
    label: "確定の文字起こし",
    hint: "話し終えたあとに置き換える字幕",
    options: [
      { value: "chrome", label: "Chrome", price: "無料", paid: false },
      { value: "gptTranscribe", label: "GPT-Transcribe", price: "話した 1 分 $0.0045", paid: true },
    ],
  },
  translation: {
    label: "翻訳と用語",
    hint: "訳文と、ヒーロー・スキル・アイテムの判定。訳文の名前を用語から作るので、いっしょに選びます",
    options: [
      { value: "chromeTranslator", label: "Chrome の翻訳 ＋ 辞書", price: "無料", paid: false },
      {
        value: "llm",
        label: "GPT-6.1 Terra",
        price: "100 万トークンで入力 $1・出力 $5",
        paid: true,
      },
    ],
  },
  furigana: {
    label: "ふりがな",
    hint: "日本語の漢字の読み",
    options: [
      { value: "lindera", label: "辞書（PC の中）", price: "無料", paid: false },
      {
        value: "llm",
        label: "GPT-6.1 Terra",
        price: "100 万トークンで入力 $1・出力 $5",
        paid: true,
      },
    ],
  },
};

/** 決まった組み合わせ。`crates/core/src/settings.rs` の `Toppings` の定数と同じ */
export const FREE: Toppings = {
  live: "chrome",
  transcription: "chrome",
  translation: "chromeTranslator",
  furigana: "lindera",
};
export const RECOMMENDED: Toppings = {
  live: "chrome",
  transcription: "gptTranscribe",
  translation: "llm",
  furigana: "llm",
};
const OPENAI: Toppings = { ...RECOMMENDED, live: "gptLiveTranscribe" };

export const PRESETS: {
  name: string;
  label: string;
  traits: string;
  price: string;
  hint: string;
  toppings: Toppings;
  recommended?: boolean;
}[] = [
  {
    name: "free",
    label: "無料",
    traits: "高速・キー不要",
    price: "無料",
    hint: "多くの字幕ソフトと同じく、Chrome の音声認識と翻訳を使います",
    toppings: FREE,
  },
  {
    name: "recommended",
    label: "おすすめ",
    traits: "高速・高精度",
    price: "約 $0.6／時間",
    hint: "途中の字幕は Chrome、確定の字幕は OpenAI の高い精度に置き換えます",
    toppings: RECOMMENDED,
    recommended: true,
  },
  {
    name: "openai",
    label: "OpenAI",
    traits: "低負荷・高精度",
    price: "約 $1／時間",
    hint: "すべて OpenAI で作ります。Chrome は使いません",
    toppings: OPENAI,
  },
];

const same = (a: Toppings, b: Toppings) =>
  (Object.keys(ITEMS) as Item[]).every((item) => a[item] === b[item]);

/** 今の選び方に合う組み合わせ。自分で選んだなら undefined */
export const presetOf = (t: Toppings) => PRESETS.find((p) => same(p.toppings, t));

/** Chrome のウィンドウを使う（途中の字幕が Chrome） */
export const usesChrome = (t: Toppings) => t.live === "chrome";

/** 発話の区切りを Chrome が決める（アプリはマイクを開かない） */
export const chromeFinals = (t: Toppings) => t.transcription === "chrome";

/** 有料の品がある */
export const anyPaid = (t: Toppings) =>
  (Object.keys(ITEMS) as Item[]).some(
    (item) => ITEMS[item].options.find((o) => o.value === t[item])?.paid,
  );

/** 今の仕組みで作れない選び方。core の `Toppings::sanitized` と同じ決まり。
 * Chrome の翻訳を使っている間は途中の字幕を Chrome から変えさせない（変えると core が翻訳を有料に直し、黙って料金がかかる） */
export function unavailable<K extends Item>(t: Toppings, item: K, value: Toppings[K]) {
  if (item === "live")
    return (chromeFinals(t) || t.translation === "chromeTranslator") && value !== "chrome";
  if (item === "translation") return !usesChrome(t) && value === "chromeTranslator";
  return false;
}
