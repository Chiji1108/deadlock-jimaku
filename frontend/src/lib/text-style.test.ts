import { expect, test } from "vitest";
import type { TextLook } from "../generated/TextLook";
import { cleared, fontCss, shadowOn, textVars, weightChoices } from "./text-style";

const standard = (): TextLook => ({
  color: null,
  outline: { width: null, color: null },
  shadow: { enabled: null, blur: null, color: null },
  background: { enabled: false, color: null, opacity: null },
});
const vars = (css?: string) =>
  Object.fromEntries((css ?? "").split("; ").map((item) => item.split(": ")));

test("標準のままなら何も書かず、CSS の既定と OBS の上書きをそのまま使う", () => {
  expect(textVars(standard())).toBeUndefined();
  expect(fontCss({ font: "A", weight: null, letterSpacing: null })).toBeUndefined();
  expect(fontCss(undefined)).toBeUndefined();
});

test("選んだ値だけを変数にする", () => {
  const look = standard();
  look.color = "#ffe066";
  look.outline = { width: 0.3, color: "#102030" };
  look.shadow.blur = 0.4;
  look.shadow.color = "#ff0000";
  expect(vars(textVars(look))).toEqual({
    "--text-color": "#ffe066",
    "--pending-color": "#ffe066c7",
    "--outline-color": "#102030",
    "--outline-width": "calc(0.3 * 1em)",
    "--shadow-blur": "calc(0.4 * 1em)",
    "--shadow-color": "#ff0000",
    "--edge": "calc(0.55 * 1em)",
  });
});

test("帯だけなら縁取りを細くして影を消す", () => {
  const look = standard();
  look.background.enabled = true;
  expect(vars(textVars(look))).toEqual({
    "--outline-width": "calc(0.08 * 1em)",
    "--text-shadow": "none",
  });
  expect(shadowOn(look)).toBe(false);
});

test("帯でも選んだ縁取りと影を優先する", () => {
  const look = standard();
  look.background.enabled = true;
  look.outline.width = 0.2;
  look.shadow.enabled = true;
  expect(vars(textVars(look))).toEqual({
    "--outline-width": "calc(0.2 * 1em)",
    "--edge": "calc(0.1 * 1em)",
  });
});

test("影を消すとぼかしと色は渡さない", () => {
  const look = standard();
  look.shadow = { enabled: false, blur: 0.5, color: "#ff0000" };
  expect(vars(textVars(look))).toEqual({ "--text-shadow": "none" });
});

test("帯の色と濃さは 1 つの色にする", () => {
  const look = standard();
  look.background = { enabled: true, color: "#112233", opacity: 0.5 };
  expect(vars(textVars(look))["--band-color"]).toBe("#11223380");
  look.background = { enabled: true, color: null, opacity: 1 };
  expect(vars(textVars(look))["--band-color"]).toBe("#000000ff");
  look.background = { enabled: true, color: "#112233", opacity: null };
  expect(vars(textVars(look))["--band-color"]).toBe("#1122339e");
});

test("ランクは --scale に比例する単位で渡す", () => {
  const look = standard();
  look.outline.width = 0.25;
  expect(vars(textVars(look, "calc(16px * var(--scale))"))["--outline-width"]).toBe(
    "calc(0.25 * calc(16px * var(--scale)))",
  );
});

test("太さと字間は p に直に書く", () => {
  expect(fontCss({ font: null, weight: 500, letterSpacing: 0 })).toBe(
    "font-weight: 500; letter-spacing: 0em",
  );
});

test("太さの選択肢に標準と選んでいる太さを足す", () => {
  expect(weightChoices([400], 700)).toEqual([400, 700]);
  expect(weightChoices([400, 900], 700, 500)).toEqual([400, 500, 700, 900]);
  expect(weightChoices(undefined, 700)).toEqual([400, 500, 700, 800, 900]);
  expect(weightChoices([], 700, 300)).toEqual([300, 400, 500, 700, 800, 900]);
});

test("標準に戻す差分は全部の項目を null にする", () => {
  expect(cleared({ color: "#ffffff", languages: {} })).toEqual({ color: null, languages: null });
});
