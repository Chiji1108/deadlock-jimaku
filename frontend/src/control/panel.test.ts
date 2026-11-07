import { beforeEach, expect, test, vi } from "vitest";
import { api } from "../lib/api";
import { panel } from "./panel.svelte";
import { addFontFaces } from "../lib/overlay-fonts";
import type { FontEntry } from "../generated/FontEntry";

vi.mock("../lib/overlay-fonts", () => ({ addFontFaces: vi.fn() }));

const pc = (...families: string[]): FontEntry[] => families.map((family) => ({ family }));

beforeEach(() => {
  panel.fonts = "loading";
  vi.restoreAllMocks();
});

function pendingFonts() {
  let resolve!: (fonts: FontEntry[]) => void;
  const pending = new Promise<FontEntry[]>((r) => (resolve = r));
  const fonts = vi.spyOn(api, "fonts").mockReturnValue(pending);
  return { fonts, resolve };
}

test("読み直しの間は前の一覧を出したままで、届いたら入れ替える", async () => {
  panel.fonts = pc("Old");
  const { resolve } = pendingFonts();
  const loading = panel.loadFonts();
  expect(panel.fonts).toEqual(pc("Old"));
  resolve(pc("New", "Old"));
  await loading;
  expect(panel.fonts).toEqual(pc("New", "Old"));
});

test("開いたあとに増えたフォントだけ読み込む", async () => {
  const add = vi.mocked(addFontFaces);
  add.mockClear();
  vi.spyOn(api, "fonts").mockResolvedValue(pc("Old"));
  await panel.loadFonts();
  await panel.loadFonts();
  expect(add).not.toHaveBeenCalled();

  vi.spyOn(api, "fonts").mockResolvedValue(pc("Installed", "Old"));
  await panel.loadFonts();
  await panel.loadFonts();
  expect(add.mock.calls).toEqual([["Installed"]]);
});

test("おすすめのフォントは、取得して初めて読み込む", async () => {
  const add = vi.mocked(addFontFaces);
  const fresh = new (panel.constructor as new () => typeof panel)();
  add.mockClear();
  const offered = { family: "Jua", languages: ["ko"], sizeBytes: 100 };
  const saved = { family: "Gaegu", languages: ["ko"] };
  vi.spyOn(api, "fonts").mockResolvedValue([offered, saved, ...pc("Old")]);
  await fresh.loadFonts();
  expect(add.mock.calls).toEqual([["Gaegu"]]);

  add.mockClear();
  vi.spyOn(api, "downloadFont").mockResolvedValue();
  vi.spyOn(api, "fonts").mockResolvedValue([{ family: "Jua", languages: ["ko"] }, saved]);
  expect(await fresh.downloadFont("Jua")).toBe(true);
  expect(add.mock.calls).toEqual([["Jua"]]);
});

test("読み直しに失敗したら前の一覧を残す", async () => {
  panel.fonts = pc("Old");
  vi.spyOn(api, "fonts").mockRejectedValue(new Error("boom"));
  await panel.loadFonts();
  expect(panel.fonts).toEqual(pc("Old"));
});

test("一覧がないときの失敗は、次に読めれば直る", async () => {
  const fonts = vi.spyOn(api, "fonts").mockRejectedValueOnce(new Error("boom"));
  await panel.loadFonts();
  expect(panel.fonts).toBe("failed");
  fonts.mockResolvedValueOnce(pc("New"));
  await panel.loadFonts();
  expect(panel.fonts).toEqual(pc("New"));
});

test("読んでいる間に呼んでも 1 回しか読まず、終われば次を読める", async () => {
  const { fonts, resolve } = pendingFonts();
  const first = panel.loadFonts();
  await panel.loadFonts();
  expect(fonts).toHaveBeenCalledTimes(1);
  resolve(pc("A"));
  await first;
  fonts.mockResolvedValue(pc("A", "B"));
  await panel.loadFonts();
  expect(fonts).toHaveBeenCalledTimes(2);
  expect(panel.fonts).toEqual(pc("A", "B"));
});
