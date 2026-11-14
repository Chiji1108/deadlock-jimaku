import { expect, test } from "vitest";
import {
  BEST,
  CHROME,
  MODES,
  keyNames,
  modeOf,
  modeStats,
  providers,
  unavailable,
} from "./engines";

test("モードの組み合わせは、そのモードとして見分ける", () => {
  for (const mode of MODES) expect(modeOf(mode.engines)?.name).toBe(mode.name);
});

test("翻訳が GPT なら用語も GPT が判定するので、用語の選択は選べず、モードも変わらない", () => {
  const t = { ...BEST, terms: "decisions" } as const;
  expect(unavailable(t, "terms", "dictionary")).toBe(true);
  expect(modeOf(t)?.name).toBe("best");
  expect(providers(t)).toEqual(["openai"]);
});

test("用語と翻訳を分けると、工程ごとの会社の鍵が要る", () => {
  const split = { ...CHROME, terms: "decisions", translation: "qwenMtFlash" } as const;
  expect(unavailable(split, "terms", "decisions")).toBe(false);
  expect(modeOf(split)).toBeUndefined();
  expect(providers(split)).toEqual(["openai", "qwen"]);
  expect(providers({ ...CHROME, translation: "qwenMtFlash" })).toEqual(["qwen"]);
});

test("OpenRouter と TypeSafe AI の工程は、その会社の鍵が要る", () => {
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

test("要る鍵は、会社の名前を並べて案内する", () => {
  expect(keyNames(["openai", "xai"])).toBe("OpenAI と xAI の API キー");
  expect(keyNames(["qwen"])).toBe("Qwen の API キー");
});

test("モードを見比べる正確さは、どのモードも同じ並びで出る", () => {
  for (const mode of MODES) {
    expect(modeStats(mode.engines).map((stat) => stat.label)).toEqual([
      "文字起こし",
      "用語",
      "ふりがな",
    ]);
  }
  // 翻訳を GPT にしたモードは、用語を GPT が翻訳と同時に判定する（辞書の一致の値を出さない）
  expect(modeStats(BEST)[1]).toEqual({ label: "用語", value: "GPT が判定" });
});
