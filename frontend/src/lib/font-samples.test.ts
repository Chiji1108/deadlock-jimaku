import { readdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";
import catalog from "../../../crates/core/src/font-catalog.json";
import { cardTarget, fontSampleName, fontSections, formatSize } from "./font-samples";

test("カタログの全部のフォントに見本があり、余分な見本がない", () => {
  const samples = readdirSync(resolve(import.meta.dirname, "../control/font-samples"));
  expect(samples.toSorted()).toEqual(
    catalog.fonts.map((font) => fontSampleName(font.family)).toSorted(),
  );
});

test("大きさは短く書く", () => {
  expect(formatSize(159_184)).toBe("0.2 MB");
  expect(formatSize(9_589_900)).toBe("9.6 MB");
  expect(formatSize(17_772_300)).toBe("18 MB");
});

test("Google Fonts はカタログの順で、この PC のフォントは名前順に混ぜる", () => {
  const fonts = [
    { family: "Zen", languages: ["ja"], downloaded: false },
    { family: "Mali", languages: ["ja"], downloaded: false },
    { family: "Jua", languages: ["ko"], downloaded: true },
    { family: "Gaegu", languages: ["ko"], sizeBytes: 100, downloaded: false },
    { family: "arial", downloaded: false },
    { family: "Nunito", downloaded: false },
  ];
  const { googleFonts, others } = fontSections(fonts, "ja");
  expect(googleFonts.map((font) => font.family)).toEqual(["Zen", "Mali"]);
  expect(others.map((font) => font.family)).toEqual(["arial", "Jua", "Nunito"]);
});

test("矢印キーは 2 列のカードを同じ列で上下し、最後の段が欠けても止まる", () => {
  const lefts = [0, 100, 0, 100, 0];
  expect(cardTarget(lefts, -1, "ArrowDown")).toBe(0);
  expect(cardTarget(lefts, 0, "ArrowDown")).toBe(2);
  expect(cardTarget(lefts, 1, "ArrowDown")).toBe(3);
  expect(cardTarget(lefts, 3, "ArrowDown")).toBeUndefined();
  expect(cardTarget(lefts, 3, "ArrowUp")).toBe(1);
  expect(cardTarget(lefts, 1, "ArrowUp")).toBe(-1);
  expect(cardTarget(lefts, 2, "ArrowRight")).toBe(3);
  expect(cardTarget(lefts, 4, "ArrowRight")).toBeUndefined();
  expect(cardTarget(lefts, 3, "ArrowLeft")).toBe(2);
});

test("検索欄では下だけがカードへ移り、左右は文字の移動に残す", () => {
  expect(cardTarget([0, 100], -1, "ArrowLeft")).toBeUndefined();
  expect(cardTarget([0, 100], -1, "ArrowRight")).toBeUndefined();
  expect(cardTarget([0, 100], -1, "ArrowUp")).toBeUndefined();
  expect(cardTarget([], -1, "ArrowDown")).toBeUndefined();
});

test("1 列なら上下はとなりのカードへ移る", () => {
  const lefts = [0, 0, 0];
  expect(cardTarget(lefts, 1, "ArrowDown")).toBe(2);
  expect(cardTarget(lefts, 1, "ArrowUp")).toBe(0);
  expect(cardTarget(lefts, 0, "ArrowUp")).toBe(-1);
});
