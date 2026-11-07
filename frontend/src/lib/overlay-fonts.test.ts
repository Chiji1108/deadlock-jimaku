import { expect, test } from "vitest";
import { endonym, fontRules } from "./overlay-fonts";

test("選んだ言語だけに変数を設定し、標準の候補へ続ける", () => {
  expect(fontRules({ ja: "Hiragino Mincho ProN" })).toBe(
    '[lang][lang="ja"] { --font: "Hiragino Mincho ProN", }',
  );
});

test("引用符と \\ を含む名前は CSS の文字列にエスケープする", () => {
  expect(fontRules({ ja: String.raw`A"B\C` })).toContain(String.raw`--font: "A\"B\\C",`);
});

test("何も選んでいなければ規則を作らない", () => {
  expect(fontRules({})).toBe("");
});

test("言語名はその言語で書く", () => {
  expect(endonym("ko")).toBe("한국어");
  expect(endonym("ru")).toBe("русский");
});

test("読めない言語コードはそのまま返す", () => {
  expect(endonym("not a language")).toBe("not a language");
});
