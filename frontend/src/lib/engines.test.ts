import { expect, test } from "vitest";
import { CHROME, MODES, STANDARD, keyNames, modeOf, providers, unavailable } from "./engines";

test("モードの組み合わせは、そのモードとして見分ける", () => {
  for (const mode of MODES) expect(modeOf(mode.engines)?.name).toBe(mode.name);
});

test("翻訳が GPT なら用語も GPT が判定するので、用語の選択は選べず、モードも変わらない", () => {
  const t = { ...STANDARD, terms: "decisions" } as const;
  expect(unavailable(t, "terms", "dictionary")).toBe(true);
  expect(modeOf(t)?.name).toBe("standard");
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
});
