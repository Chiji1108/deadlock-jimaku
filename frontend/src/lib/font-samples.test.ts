import { readdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";
import catalog from "../../../crates/core/src/font-catalog.json";
import { fontSampleName, formatSize } from "./font-samples";

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
