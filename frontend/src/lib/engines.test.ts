import { expect, test } from "vitest";
import {
  BEST,
  BETA,
  CHROME,
  CLOUD,
  ITEMS,
  MODES,
  keyNames,
  modeOf,
  modeStats,
  providers,
  shown,
  unavailable,
  usesServer,
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

test("サーバーに頼む工程は、会社の鍵を使わない", () => {
  for (const item of ["transcription", "terms", "translation", "furigana"] as const) {
    const server = { ...CHROME, [item]: "server" };
    expect(providers(server)).toEqual([]);
    expect(usesServer(server)).toBe(true);
  }
  expect(usesServer(CHROME)).toBe(false);
  // GPT が翻訳と同時に用語を判定するなら、用語はサーバーに頼まない
  expect(usesServer({ ...CHROME, terms: "server", translation: "gpt61Terra" })).toBe(false);
  // 無料でサーバーに頼めるのは「おすすめ」の中身だけ。途中の字幕には、サーバーの選択肢がない
  expect(ITEMS.live.options.map((o) => o.value)).not.toContain("server");
  // 「クラウド」は、途中の字幕だけ Qwen の自分のキーが要る。ほかは「おすすめ」と同じで、サーバーに頼む
  const named = MODES.filter((m) => usesServer(m.engines)).map((m) => m.name);
  expect(named).toEqual(["beta", "cloud"]);
  expect(providers(BETA)).toEqual([]);
  expect(providers(CLOUD)).toEqual(["qwen"]);
});

test("要る鍵は、会社の名前を並べて案内する", () => {
  expect(keyNames(["openai", "xai"])).toBe("OpenAI と xAI の API キー");
  expect(keyNames(["qwen"])).toBe("Qwen の API キー");
});

test("モードを見比べる正確さは、どのモードも同じ並びで出る", () => {
  const models = {
    transcribe: { id: "qwen-audio", label: "" },
    decide: { id: "pplx-decider", label: "" },
  };
  // サーバーの今のモデルが分からない間は、サーバーに頼む工程を出さない
  expect(modeStats(BETA)).toEqual([]);
  for (const mode of MODES) {
    expect(modeStats(mode.engines, models).map((stat) => stat.label)).toEqual([
      "文字起こし",
      "用語",
      "ふりがな",
    ]);
  }
  // 翻訳を GPT にしたモードは、用語を GPT が翻訳と同時に判定する（辞書の一致の値を出さない）
  expect(modeStats(BEST)[1]).toEqual({ label: "用語", value: "GPT が判定", grade: "◎" });
  // 辞書の一致だけの Chrome は △、Chrome 内蔵の AI で確かめる Chrome+ は ○
  const terms = (name: string) =>
    modeStats(MODES.find((m) => m.name === name)!.engines, models)[1]!.grade;
  expect([terms("beta"), terms("chrome"), terms("chromePlus")]).toEqual(["◎", "△", "○"]);
});

test("サーバーの選択肢は、サーバーが今使っているモデルの名前と数字を出す", () => {
  const server = <K extends "transcription" | "translation">(item: K) =>
    ITEMS[item].options.find((o) => o.value === "server")!;
  const models = {
    transcribe: { id: "qwen-audio", label: "Qwen-Audio-3.1-ASR-Flash" },
    translate: { id: "new-model", label: "新しいモデル" },
  };
  // 同じモデルの選択肢があれば、その数字
  const qwen = ITEMS.transcription.options.find((o) => o.value === "qwenAudio")!;
  expect(shown("transcription", server("transcription"), models)).toMatchObject({
    model: "Qwen-Audio-3.1-ASR-Flash",
    accuracy: qwen.accuracy,
    speed: qwen.speed,
  });
  // アプリの知らないモデルは、名前だけ
  const unknown = shown("translation", server("translation"), models);
  expect(unknown).toMatchObject({ model: "新しいモデル" });
  expect(unknown.accuracy).toBeUndefined();
  // 分からない間は、何も足さない。サーバーでない選択肢は、そのまま
  expect(shown("transcription", server("transcription"), undefined)).toBe(server("transcription"));
  expect(shown("transcription", qwen, models)).toBe(qwen);
  // サーバーの選択肢に、数字は書かない（サーバーがモデルを替えると合わなくなる）
  for (const item of ["transcription", "terms", "translation", "furigana"] as const) {
    const option = ITEMS[item].options.find((o) => o.value === "server")!;
    expect([option.accuracy, option.speed]).toEqual([undefined, undefined]);
  }
});
