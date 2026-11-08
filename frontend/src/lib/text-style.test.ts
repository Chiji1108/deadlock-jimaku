import { expect, test } from "vitest";
import type { TextLook } from "../generated/TextLook";
import {
  changedDetails,
  cleared,
  fontCss,
  shadowOn,
  syntheticBold,
  textVars,
  weightChoices,
} from "./text-style";

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
    "--pending-color": "#cbb75a",
    "--outline-color": "#102030",
    "--outline-width": "calc(0.3 * 1em)",
    "--shadow-blur": "calc(0.4 * 1em)",
    "--shadow-color": "#ff0000",
    "--edge": "calc(0.15 * 1em + 0.4 * 1em)",
  });
});

test("白い字と黒い縁取りを選べば、確定前の字は標準と同じ灰色で、用語の色は変えない", () => {
  const look = standard();
  look.color = "#ffffff";
  look.outline.color = "#000000";
  expect(vars(textVars(look))).toEqual({
    "--text-color": "#ffffff",
    "--pending-color": "#c8c8c8",
    "--outline-color": "#000000",
  });
});

test("縁取りより暗い字では、用語の色の明るさを反転し、確定前の字は縁取りの色へ寄せる", () => {
  const look = standard();
  look.color = "#000000";
  look.outline.color = "#ffffff";
  const inverted = vars(textVars(look));
  expect(inverted["--pending-color"]).toBe("#373737");
  expect(inverted["--hero-color"]).toBe("#81004c");
  expect(inverted["--ability-color"]).toBe("#00354b");
  expect(Object.keys(inverted).filter((name) => name.endsWith("-color"))).toHaveLength(8);
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
    "--edge": "calc(0.1 * 1em + 0 * 1em)",
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

test("ランクは標準と同じく、ぼかしだけを --scale に比例させる", () => {
  const look = standard();
  look.outline.width = 0.25;
  look.shadow.blur = 0.5;
  const rank = vars(textVars(look, "calc(16px * var(--scale))"));
  expect(rank["--outline-width"]).toBe("calc(0.25 * 1em)");
  expect(rank["--shadow-blur"]).toBe("calc(0.5 * calc(16px * var(--scale)))");
  expect(rank["--edge"]).toBe("calc(0.125 * 1em + 0.5 * calc(16px * var(--scale)))");
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

test("太字を合成するのは、600 以上を求めて 600 以上の面がないときだけ", () => {
  expect(syntheticBold([400], 700)).toBe(true);
  expect(syntheticBold([400, 500], 600)).toBe(true);
  expect(syntheticBold([400], 500)).toBe(false);
  expect(syntheticBold([400, 600], 800)).toBe(false);
  expect(syntheticBold([400, 900], 700)).toBe(false);
  expect(syntheticBold([], 700)).toBe(false);
  expect(syntheticBold(undefined, 700)).toBe(false);
});

test("標準に戻す差分は全部の項目を null にする", () => {
  expect(cleared({ color: "#ffffff", languages: {} })).toEqual({ color: null, languages: null });
});

test("「詳しく」の中で変えている項目を数える（隠れている項目は数えない）", () => {
  const look = standard();
  expect(changedDetails(look, [])).toBe(0);
  look.color = "#000000";
  look.outline.color = "#ffffff";
  expect(changedDetails(look, [])).toBe(2);
  // 帯がないときの帯の色、影を消したときのぼかしは効かない
  look.background.color = "#ffffff";
  look.shadow = { enabled: false, blur: 0.3, color: null };
  expect(changedDetails(look, [])).toBe(3);
  look.background.enabled = true;
  look.shadow.enabled = true;
  expect(changedDetails(look, [])).toBe(5);
});

test("字幕は行間と言語ごとの字間、ランクは字間も数える", () => {
  const look = standard();
  // 字幕: 行間と、出している言語の字間（ない言語は undefined）
  expect(changedDetails(look, [1.8, 0.05, undefined])).toBe(2);
  expect(changedDetails(look, [null, null])).toBe(0);
  // ランク: 字間
  expect(changedDetails(look, [0.02])).toBe(1);
});
