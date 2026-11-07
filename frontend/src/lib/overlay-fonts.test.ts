import { expect, test } from "vitest";
import { applyOverlayFonts, fontGroup } from "./overlay-fonts";

test("言語は lang.css と同じ区分に分かれる", () => {
  expect(fontGroup("ja")).toBe("ja");
  expect(fontGroup("zh-Hant")).toBe("zh-Hant");
  expect(fontGroup("en")).toBe("latin");
  expect(fontGroup("ru")).toBe("latin");
  expect(fontGroup("es-419")).toBe("latin");
});

function fakeStyle() {
  const values = new Map<string, string>();
  return {
    values,
    setProperty: (name: string, value: string | null) => void values.set(name, value ?? ""),
    removeProperty: (name: string) => (values.delete(name), ""),
  };
}

test("選んだ言語だけに変数を設定し、標準の候補へ続ける", () => {
  const style = fakeStyle();
  applyOverlayFonts({ ja: "Hiragino Mincho ProN" }, style);
  expect([...style.values]).toEqual([["--font-ja", '"Hiragino Mincho ProN",']]);
});

test("引用符と \\ を含む名前は CSS の文字列にエスケープする", () => {
  const style = fakeStyle();
  applyOverlayFonts({ ja: String.raw`A"B\C` }, style);
  expect(style.values.get("--font-ja")).toBe(String.raw`"A\"B\\C",`);
});

test("標準に戻すと変数を消す", () => {
  const style = fakeStyle();
  applyOverlayFonts({ ja: "Hiragino Sans", latin: "Georgia" }, style);
  applyOverlayFonts({ latin: "Georgia" }, style);
  expect([...style.values.keys()]).toEqual(["--font-latin"]);
});
