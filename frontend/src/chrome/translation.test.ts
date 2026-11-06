import { expect, test, vi } from "vitest";
import type { ChromeTask } from "../generated/ChromeTask";
import type { Translator } from "./speech";
import { markNames, Translation } from "./translation";

const magazine = { ref: "item:1", en: "Extended Magazine" };
const task = (id: string, text: string, final: boolean): ChromeTask => ({
  id,
  text,
  terms: [],
  final,
});

test("訳文に残った英語名を用語にする", () => {
  expect(markNames("Buy Extended Magazine first", [magazine])).toEqual([
    { t: "text", text: "Buy " },
    { t: "entity", ref: "item:1" },
    { t: "text", text: " first" },
  ]);
  expect(markNames("Buy a clip", [])).toEqual([{ t: "text", text: "Buy a clip" }]);
});

test("別の単語の一部は名前にしない", () => {
  const rem = { ref: "hero:1", en: "Rem" };
  expect(markNames("Remember", [rem])).toEqual([{ t: "text", text: "Remember" }]);
  expect(markNames("Rem's Remember", [rem])).toEqual([
    { t: "entity", ref: "hero:1" },
    { t: "text", text: "'s Remember" },
  ]);
});

test("同じ位置からは長い名前を取る", () => {
  const short = { ref: "item:2", en: "Extended" };
  expect(markNames("Extended Magazine", [short, magazine])).toEqual([
    { t: "entity", ref: "item:1" },
  ]);
});

test("訳が空なら用語もない", () => {
  expect(markNames("", [magazine])).toEqual([]);
});

/** 手で終わらせられる翻訳 */
function pendingTranslator() {
  const finish: ((text: string) => void)[] = [];
  const translator: Translator = {
    translate: vi.fn(() => new Promise<string>((resolve) => finish.push(resolve))),
  };
  return { translator, finish };
}

test("途中の文は最新だけを訳し、確定の文はすべて訳す", async () => {
  const { translator, finish } = pendingTranslator();
  const send = vi.fn();
  const translation = new Translation(new Map([["en", translator]]), send);
  translation.onTask(task("a", "ヘ", false));
  translation.onTask(task("a", "ヘイ", false));
  translation.onTask(task("a", "ヘイズ", false));
  expect(translator.translate).toHaveBeenCalledTimes(1);
  finish[0]!("He");
  await vi.waitFor(() => expect(translator.translate).toHaveBeenCalledTimes(2));
  expect(translator.translate).toHaveBeenLastCalledWith("ヘイズ");
  expect(send).toHaveBeenCalledWith({
    id: "a",
    final: false,
    translations: { en: [{ t: "text", text: "He" }] },
  });

  translation.onTask(task("a", "ヘイズが強い", true));
  expect(translator.translate).toHaveBeenCalledTimes(3);
  finish[2]!("Haze is strong");
  await vi.waitFor(() =>
    expect(send).toHaveBeenLastCalledWith({
      id: "a",
      final: true,
      translations: { en: [{ t: "text", text: "Haze is strong" }] },
    }),
  );
});

test("確定したら、待っていた同じ発話の途中の文は訳さない", async () => {
  const { translator, finish } = pendingTranslator();
  const translation = new Translation(new Map([["en", translator]]), vi.fn());
  translation.onTask(task("a", "ヘ", false));
  translation.onTask(task("a", "ヘイ", false));
  translation.onTask(task("a", "ヘイズ", true));
  finish[0]!("He");
  await Promise.resolve();
  expect(translator.translate).not.toHaveBeenCalledWith("ヘイ");
});

test("訳せなかった言語は省き、ページに翻訳がなくても返事をする", async () => {
  const failing: Translator = { translate: () => Promise.reject(new Error("no")) };
  const send = vi.fn();
  new Translation(new Map([["en", failing]]), send).onTask(task("a", "ヘイズ", true));
  await vi.waitFor(() =>
    expect(send).toHaveBeenCalledWith({ id: "a", final: true, translations: {} }),
  );
  send.mockClear();
  new Translation(new Map(), send).onTask(task("b", "ヘイズ", true));
  await vi.waitFor(() =>
    expect(send).toHaveBeenCalledWith({ id: "b", final: true, translations: {} }),
  );
});
