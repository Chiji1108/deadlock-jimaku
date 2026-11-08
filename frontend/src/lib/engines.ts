// 字幕の作り方。操作パネルのモードの欄と LP のモードの節が、同じ並びと言い方で出す
import type { SpeechEngine } from "../generated/SpeechEngine";

export const ENGINES: {
  value: SpeechEngine;
  label: string;
  traits: string;
  price: string;
  hint: string;
  /** OpenAI の API キーかギフトコードが要る */
  needsKey: boolean;
  recommended?: boolean;
}[] = [
  {
    value: "chrome",
    label: "Chrome",
    traits: "高速・キー不要",
    price: "無料",
    hint: "多くの字幕ソフトと同じく、Chrome の音声認識を使います",
    needsKey: false,
  },
  {
    value: "hybrid",
    label: "ハイブリッド",
    traits: "高速・高精度",
    price: "約 $0.6／時間",
    hint: "途中の字幕は Chrome、確定の字幕は OpenAI の高い精度に置き換えます",
    needsKey: true,
    recommended: true,
  },
  {
    value: "openai",
    label: "OpenAI",
    traits: "低負荷・高精度",
    price: "約 $1／時間",
    hint: "すべて OpenAI で作ります。Chrome は使いません",
    needsKey: true,
  },
];
