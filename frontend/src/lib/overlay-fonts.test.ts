import { expect, test } from "vitest";
import { endonym, fontRules, fontSample } from "./overlay-fonts";

test("選んだ言語だけに変数を設定し、標準の候補へ続ける", () => {
  expect(fontRules({ ja: "Hiragino Mincho ProN" })).toBe(
    [
      '@import url("/api/fonts/Hiragino%20Mincho%20ProN/faces.css");',
      '[lang][lang="ja"] { --font: "Hiragino Mincho ProN", }',
    ].join("\n"),
  );
});

test("@import はシートの先頭に、同じフォントは 1 つだけ書く", () => {
  const rules = fontRules({ ja: "Meiryo", ko: "Meiryo", en: "Arial" }).split("\n");
  expect(rules.filter((rule) => rule.startsWith("@import"))).toEqual([
    '@import url("/api/fonts/Meiryo/faces.css");',
    '@import url("/api/fonts/Arial/faces.css");',
  ]);
  expect(rules.slice(0, 2).every((rule) => rule.startsWith("@import"))).toBe(true);
});

test("URL に書けない文字はパスの 1 区切りに収める", () => {
  expect(fontRules({ ja: 'A/B"C' })).toContain('url("/api/fonts/A%2FB%22C/faces.css")');
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

test("見本は言語名で、日本語だけかなと漢字を混ぜる", () => {
  expect(fontSample("ko")).toBe("한국어");
  expect(fontSample("ja")).toMatch(/\p{Script=Hiragana}.*\p{Script=Katakana}.*\p{Script=Han}/u);
});
