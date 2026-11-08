import { expect, test } from "vitest";
import { invertLightness, luminance, mix } from "./color";

const contrast = (a: string, b: string) => {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
};

test("明るさを反転すると、白と黒へのコントラスト比が入れ替わる", () => {
  expect(invertLightness("#ffffff")).toBe("#000000");
  expect(invertLightness("#000000")).toBe("#ffffff");
  expect(contrast(invertLightness("#c8c8c8"), "#ffffff")).toBeCloseTo(
    contrast("#c8c8c8", "#000000"),
    0,
  );
});

test("明るい色を反転しても、白い縁取りに対して読める濃さで色味が残る", () => {
  for (const color of ["#ff7ab6", "#8fd8ff"]) {
    const dark = invertLightness(color);
    expect(contrast(dark, "#ffffff")).toBeGreaterThan(7);
    expect(dark).not.toMatch(/^#(..)\1\1$/);
  }
});

test("sRGB で混ぜる", () => {
  expect(mix("#ffffff", "#000000", 0.5)).toBe("#808080");
  expect(mix("#ff0000", "#0000ff", 0)).toBe("#ff0000");
});
