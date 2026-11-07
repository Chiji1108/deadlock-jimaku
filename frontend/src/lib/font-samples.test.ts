import { readdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";
import catalog from "../../../crates/core/src/font-catalog.json";
import { fontSampleName, fontSections, formatSize } from "./font-samples";

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

test("おすすめはカタログの順で、この PC のフォントは名前順に混ぜる", () => {
  const fonts = [
    { family: "Zen", languages: ["ja"], downloaded: false },
    { family: "Mali", languages: ["ja"], downloaded: false },
    { family: "Jua", languages: ["ko"], downloaded: true },
    { family: "Gaegu", languages: ["ko"], sizeBytes: 100, downloaded: false },
    { family: "arial", downloaded: false },
    { family: "Nunito", downloaded: false },
  ];
  const { recommended, others } = fontSections(fonts, "ja");
  expect(recommended.map((font) => font.family)).toEqual(["Zen", "Mali"]);
  expect(others.map((font) => font.family)).toEqual(["arial", "Jua", "Nunito"]);
});
