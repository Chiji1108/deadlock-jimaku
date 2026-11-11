import { expect, test } from "vitest";
import { CHROME, MODES, keyNames, modeOf, providers, unavailable } from "./engines";

/** 翻訳も用語も GPT に任せる組み合わせ（モードにはない） */
const GPT = {
  ...CHROME,
  transcription: "gptTranscribe",
  translation: "gpt61Terra",
  furigana: "llm",
} as const;

test("モードの組み合わせは、そのモードとして見分ける", () => {
  for (const mode of MODES) expect(modeOf(mode.engines)?.name).toBe(mode.name);
});

test("翻訳が GPT なら用語も GPT が判定するので、用語の選択は選べない", () => {
  const t = { ...GPT, terms: "decisions" } as const;
  expect(unavailable(t, "terms", "dictionary")).toBe(true);
  expect(modeOf(t)).toBeUndefined();
  expect(providers(t)).toEqual(["openai"]);
});

test("用語と翻訳を分けると、工程ごとの会社の鍵が要る", () => {
  const split = { ...CHROME, terms: "decisions", translation: "qwenMtFlash" } as const;
  expect(unavailable(split, "terms", "decisions")).toBe(false);
  expect(modeOf(split)).toBeUndefined();
  expect(providers(split)).toEqual(["openai", "qwen"]);
  expect(providers({ ...CHROME, translation: "qwenMtFlash" })).toEqual(["qwen"]);
});

test("OpenRouter と TypeSafe AI の工程は、その会社の鍵が要る（ギフトコードでは足りない）", () => {
  const mixed = {
    ...CHROME,
    transcription: "maiTranscribe2",
    terms: "jev",
    translation: "hyMt2",
  } as const;
  expect(providers(mixed)).toEqual(["openrouter", "typesafe"]);
  expect(keyNames(providers(mixed))).toBe("OpenRouter と TypeSafe AI の API キー");
  // PC の中の文字起こしは鍵が要らない
  expect(providers({ ...CHROME, transcription: "haasJa" })).toEqual([]);
});

test("ギフトコードで足りる会社があるときだけ、ギフトコードを案内する", () => {
  expect(keyNames(["openai", "xai"])).toBe("OpenAI と xAI の API キーかギフトコード");
  expect(keyNames(["qwen"])).toBe("Qwen の API キー");
  // ベータのコードで使える会社は、ギフトコードとは案内しない（人からもらったコードでは使えない）
  expect(keyNames(["qwen", "openrouter"])).toBe("Qwen と OpenRouter の API キー");
});
