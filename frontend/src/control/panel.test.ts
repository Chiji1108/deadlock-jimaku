import { beforeEach, expect, test, vi } from "vitest";
import { api } from "../lib/api";
import { panel, type FontTarget } from "./panel.svelte";
import { addFontFaces } from "../lib/overlay-fonts";
import type { FontEntry } from "../generated/FontEntry";
import type { SettingsView } from "../generated/SettingsView";

vi.mock("../lib/overlay-fonts", () => ({ addFontFaces: vi.fn() }));

const pc = (...families: string[]): FontEntry[] =>
  families.map((family) => ({ family, downloaded: false }));

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

const freshPanel = () => new (panel.constructor as new () => typeof panel)();

const offered = { family: "Jua", languages: ["ko"], sizeBytes: 100, downloaded: false };
const saved = { family: "Jua", languages: ["ko"], downloaded: true };

const KO: FontTarget = { kind: "subtitle", language: "ko" };

function withFonts(fonts: Record<string, string>) {
  const view = { settings: { subtitleDisplay: { fonts } } } as unknown as SettingsView;
  const next = freshPanel();
  next.view = view;
  const update = vi.spyOn(next, "update").mockResolvedValue(true);
  return { next, update };
}

test("最初の一覧では、PC にないダウンロード済みのフォントだけ読み込む", async () => {
  const add = vi.mocked(addFontFaces);
  const fresh = freshPanel();
  add.mockClear();
  const onPc = { family: "Gaegu", languages: ["ko"], downloaded: false };
  vi.spyOn(api, "fonts").mockResolvedValue([saved, onPc, ...pc("Old")]);
  await fresh.loadFonts();
  expect(add.mock.calls).toEqual([["Jua"]]);
});

test("Google Fonts のフォントは、取得して初めて読み込み、その言語に適用する", async () => {
  const add = vi.mocked(addFontFaces);
  const { next, update } = withFonts({});
  add.mockClear();
  vi.spyOn(api, "fonts").mockResolvedValue([offered, ...pc("Old")]);
  await next.loadFonts();
  expect(add).not.toHaveBeenCalled();

  vi.spyOn(api, "downloadFont").mockResolvedValue();
  vi.spyOn(api, "fonts").mockResolvedValue([saved, ...pc("Old")]);
  await next.downloadFont("Jua", KO);
  expect(add.mock.calls).toEqual([["Jua"]]);
  expect(update).toHaveBeenCalledWith({ subtitleDisplay: { fonts: { ko: "Jua" } } });
});

test("ダウンロードの間は進行中の印があり、終われば外れる", async () => {
  const { next } = withFonts({});
  let finish!: () => void;
  vi.spyOn(api, "downloadFont").mockReturnValue(new Promise((r) => (finish = r)));
  vi.spyOn(api, "fonts").mockResolvedValue([saved]);
  const download = next.downloadFont("Jua", KO);
  expect(next.downloadingFonts.has("Jua")).toBe(true);
  finish();
  await download;
  expect(next.downloadingFonts.has("Jua")).toBe(false);
});

test("ダウンロードを待つ間に選び直したフォントは上書きしない", async () => {
  const { next, update } = withFonts({});
  let finish!: () => void;
  vi.spyOn(api, "downloadFont").mockReturnValue(new Promise((r) => (finish = r)));
  vi.spyOn(api, "fonts").mockResolvedValue([saved]);
  const download = next.downloadFont("Jua", KO);
  next.settings!.subtitleDisplay.fonts.ko = "Gaegu";
  finish();
  await download;
  expect(update).not.toHaveBeenCalled();
});

test("ランクのフォントを取得したら、ランクにだけ適用する", async () => {
  const { next, update } = withFonts({});
  vi.spyOn(api, "downloadFont").mockResolvedValue();
  vi.spyOn(api, "fonts").mockResolvedValue([saved]);
  await next.downloadFont("Jua", { kind: "rank" });
  expect(update).toHaveBeenCalledWith({ rankFont: "Jua" });
});

test("失敗したダウンロードは適用せず、印を外す", async () => {
  const { next, update } = withFonts({});
  vi.spyOn(api, "downloadFont").mockRejectedValue(new Error("boom"));
  await next.downloadFont("Jua", KO);
  expect(update).not.toHaveBeenCalled();
  expect(next.downloadingFonts.has("Jua")).toBe(false);
});

test("読み込みの途中で終わったダウンロードは、その読み込みのあとに読み直して反映する", async () => {
  const { next } = withFonts({});
  const { fonts, resolve } = pendingFonts();
  const running = next.loadFonts();
  vi.spyOn(api, "downloadFont").mockResolvedValue();
  const download = next.downloadFont("Jua", KO);
  await vi.waitFor(() => expect(api.downloadFont).toHaveBeenCalled());
  await Promise.resolve();

  fonts.mockResolvedValue([saved]);
  resolve([offered]);
  await running;
  await download;
  expect(fonts).toHaveBeenCalledTimes(2);
  expect(next.fonts).toEqual([saved]);
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
  const second = panel.loadFonts();
  expect(fonts).toHaveBeenCalledTimes(1);
  resolve(pc("A"));
  await Promise.all([first, second]);
  fonts.mockResolvedValue(pc("A", "B"));
  await panel.loadFonts();
  expect(fonts).toHaveBeenCalledTimes(2);
  expect(panel.fonts).toEqual(pc("A", "B"));
});
