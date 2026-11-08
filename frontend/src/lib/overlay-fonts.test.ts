import { expect, test, vi } from "vitest";
import {
  addFontFaces,
  endonym,
  faceWeights,
  fontSample,
  fontImports,
  fontVar,
  fontWeights,
} from "./overlay-fonts";

test("--font の値は末尾のカンマで標準の候補へ続け、選んでいなければ書かない", () => {
  expect(fontVar("Hiragino Mincho ProN")).toBe('"Hiragino Mincho ProN",');
  expect(fontVar(null)).toBeUndefined();
  expect(fontVar("")).toBeUndefined();
});

test("引用符と \\ を含む名前は CSS の文字列にエスケープする", () => {
  expect(fontVar(String.raw`A"B\C`)).toBe(String.raw`"A\"B\\C",`);
});

test("読み込みは同じフォントを 1 つだけ書き、選んでいない言語は飛ばす", () => {
  expect(fontImports(["Meiryo", null, "Meiryo", "Arial"]).split("\n")).toEqual([
    '@import url("/api/fonts/Meiryo/faces.css");',
    '@import url("/api/fonts/Arial/faces.css");',
  ]);
  expect(fontImports([null])).toBe("");
});

test("URL に書けない文字はパスの 1 区切りに収める", () => {
  expect(fontImports(['A/B"C'])).toBe('@import url("/api/fonts/A%2FB%22C/faces.css");');
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

test("フォントの読み込みは、名前を URL に入れた faces.css を足す", () => {
  const links: { href?: string }[] = [];
  vi.stubGlobal("document", {
    createElement: () => ({}),
    head: { append: (link: { href?: string }) => links.push(link) },
  });
  addFontFaces("A B");
  addFontFaces("C");
  expect(links.map((link) => link.href)).toEqual([
    "/api/fonts/A%20B/faces.css",
    "/api/fonts/C/faces.css",
  ]);
  vi.unstubAllGlobals();
});

test("太さは faces.css から読み、読めなければ reject", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string) =>
      url.includes("/A/")
        ? { ok: false, status: 404 }
        : { ok: true, text: async () => "font-weight: 400 400;" },
    ),
  );
  await expect(fontWeights("A")).rejects.toThrow("404");
  await expect(fontWeights("B")).resolves.toEqual([400]);
  vi.unstubAllGlobals();
});

test("faces.css から太さを読む", () => {
  const css = (min: number, max: number) =>
    `@font-face { font-family: "A"; font-weight: ${min} ${max}; font-display: swap; src: url("A"); }`;
  expect(faceWeights([css(400, 400), css(700, 700)].join("\n"))).toEqual([400, 700]);
  expect(faceWeights(css(350, 350))).toEqual([350]);
  expect(faceWeights(css(250, 620))).toEqual([300, 400, 500, 600]);
  expect(faceWeights("")).toEqual([]);
});
