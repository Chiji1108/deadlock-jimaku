import { afterEach, expect, test, vi } from "vitest";
import { judgeAvailability, prepareJudge } from "./judge";

afterEach(() => vi.unstubAllGlobals());

/** Prompt API の代わり。`answers` を順に返し、聞かれた入力と答えの決まりを覚える */
function fakeModel(answers: string[], availability = "available") {
  const asked: { input: Record<string, unknown>; numbers: string[] }[] = [];
  const session = {
    clone: async () => session,
    destroy: () => {},
    prompt: async (input: string, options: { responseConstraint: unknown }) => {
      const schema = options.responseConstraint as { properties: { choice: { enum: string[] } } };
      asked.push({ input: JSON.parse(input), numbers: schema.properties.choice.enum });
      const answer = answers.shift();
      if (answer === undefined) throw new Error("no answer");
      return answer;
    },
  };
  vi.stubGlobal("LanguageModel", {
    availability: async () => availability,
    create: async () => session,
  });
  return asked;
}

const task = {
  id: "seg-1",
  state: "Transcript: ヘイズのウルトで倒した",
  questions: [
    {
      name: "c0",
      instructions: "What does ヘイズ mean?",
      choices: [
        { value: "hero:13", description: "the hero Haze" },
        { value: "none", description: "None of these" },
      ],
    },
    {
      name: "c1",
      instructions: "What does ウルト mean?",
      choices: [
        { value: "ability:4", description: "Bullet Dance, the ultimate of Haze" },
        { value: "none", description: "None of these" },
      ],
    },
  ],
};

test("選択肢を番号で見せて、選ばれた番号を選択肢の名前に戻す", async () => {
  const asked = fakeModel(['{"choice":"1"}', '{"choice":"2"}']);
  const judge = await prepareJudge(() => {});
  expect(await judge.decide(task)).toEqual({
    id: "seg-1",
    choices: { c0: "hero:13", c1: "none" },
  });
  // 答えの決まりには番号だけを入れる（選択肢の名前や日本語を入れると、Prompt API がエラーを返す）
  expect(asked[0]).toEqual({
    input: {
      state: task.state,
      question: "What does ヘイズ mean?",
      choices: { "1": "the hero Haze", "2": "None of these" },
    },
    numbers: ["1", "2"],
  });
});

test("答えられなかった問いは省く（core は辞書の一致のままにする）", async () => {
  fakeModel(["not json", '{"choice":"1"}']);
  const judge = await prepareJudge(() => {});
  expect(await judge.decide(task)).toEqual({ id: "seg-1", choices: { c1: "ability:4" } });
  // モデルが落ちても、答えは返す
  expect(await judge.decide(task)).toEqual({ id: "seg-1", choices: {} });
});

test("使えるかは、Prompt API があるかと、その答えで決まる", async () => {
  expect(await judgeAvailability()).toBe("unavailable");
  fakeModel([], "downloading");
  expect(await judgeAvailability()).toBe("downloadable");
  fakeModel([], "available");
  expect(await judgeAvailability()).toBe("available");
  fakeModel([], "unavailable");
  expect(await judgeAvailability()).toBe("unavailable");
});
