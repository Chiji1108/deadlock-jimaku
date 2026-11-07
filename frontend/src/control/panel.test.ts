import { beforeEach, expect, test, vi } from "vitest";
import { api } from "../lib/api";
import { panel } from "./panel.svelte";

beforeEach(() => {
  panel.fonts = "loading";
  vi.restoreAllMocks();
});

function pendingFonts() {
  let resolve!: (families: string[]) => void;
  const pending = new Promise<string[]>((r) => (resolve = r));
  const fonts = vi.spyOn(api, "fonts").mockReturnValue(pending);
  return { fonts, resolve };
}

test("読み直しの間は前の一覧を出したままで、届いたら入れ替える", async () => {
  panel.fonts = ["Old"];
  const { resolve } = pendingFonts();
  const loading = panel.loadFonts();
  expect(panel.fonts).toEqual(["Old"]);
  resolve(["New", "Old"]);
  await loading;
  expect(panel.fonts).toEqual(["New", "Old"]);
});

test("読み直しに失敗したら前の一覧を残す", async () => {
  panel.fonts = ["Old"];
  vi.spyOn(api, "fonts").mockRejectedValue(new Error("boom"));
  await panel.loadFonts();
  expect(panel.fonts).toEqual(["Old"]);
});

test("一覧がないときの失敗は、次に読めれば直る", async () => {
  const fonts = vi.spyOn(api, "fonts").mockRejectedValueOnce(new Error("boom"));
  await panel.loadFonts();
  expect(panel.fonts).toBe("failed");
  fonts.mockResolvedValueOnce(["New"]);
  await panel.loadFonts();
  expect(panel.fonts).toEqual(["New"]);
});

test("読んでいる間に呼んでも 1 回しか読まず、終われば次を読める", async () => {
  const { fonts, resolve } = pendingFonts();
  const first = panel.loadFonts();
  await panel.loadFonts();
  expect(fonts).toHaveBeenCalledTimes(1);
  resolve(["A"]);
  await first;
  fonts.mockResolvedValue(["A", "B"]);
  await panel.loadFonts();
  expect(fonts).toHaveBeenCalledTimes(2);
  expect(panel.fonts).toEqual(["A", "B"]);
});
