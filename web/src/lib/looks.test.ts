import { expect, test } from "vitest";
import catalog from "../../../crates/core/src/font-catalog.json";
import { LOOKS } from "./looks";

test("見本のフォントは、アプリのカタログで日本語に出るフォント", () => {
  const fonts = LOOKS.flatMap(({ fontStyle }) => (fontStyle.font ? [fontStyle.font] : []));
  expect(fonts.length).toBeGreaterThan(0);
  for (const font of fonts) {
    expect(catalog.fonts.find((entry) => entry.family === font)?.languages).toContain("ja");
  }
});
