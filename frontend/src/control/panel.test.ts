import { beforeEach, expect, test, vi } from "vitest";
import { api } from "../lib/api";
import { panel, type FontTarget } from "./panel.svelte";
import { addFontFaces, fontWeights } from "../lib/overlay-fonts";
import type { FontEntry } from "../generated/FontEntry";
import type { SettingsView } from "../generated/SettingsView";
import { BETA, CHROME } from "../lib/engines";

vi.mock("../lib/overlay-fonts", () => ({ addFontFaces: vi.fn(), fontWeights: vi.fn() }));
vi.mock("../lib/ws", () => ({ connect: vi.fn(() => () => {}) }));

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
  const languages = Object.fromEntries(
    Object.entries(fonts).map(([lang, font]) => [lang, { font }]),
  );
  const view = { settings: { subtitleStyle: { languages } } } as unknown as SettingsView;
  const next = freshPanel();
  next.view = view;
  const update = vi.spyOn(next, "update").mockResolvedValue(true);
  return { next, update };
}

const never = () => new Promise<never>(() => {});

test("操作パネルを開いたときに一覧を読む（あとで入れたフォントを最初の一覧に入れない）", () => {
  const fresh = freshPanel();
  for (const name of ["settings", "languages", "devices", "gift", "share"] as const)
    vi.spyOn(api, name).mockImplementation(never);
  const fonts = vi.spyOn(api, "fonts").mockImplementation(never);
  const stop = fresh.start();
  expect(fonts).toHaveBeenCalledTimes(1);
  stop();
});

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
  expect(update).toHaveBeenCalledWith({
    subtitleStyle: { languages: { ko: { font: "Jua" } } },
  });
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
  next.settings!.subtitleStyle.languages.ko = { font: "Gaegu", weight: null, letterSpacing: null };
  finish();
  await download;
  expect(update).not.toHaveBeenCalled();
});

test("ランクのフォントを取得したら、ランクにだけ適用する", async () => {
  const { next, update } = withFonts({});
  vi.spyOn(api, "downloadFont").mockResolvedValue();
  vi.spyOn(api, "fonts").mockResolvedValue([saved]);
  await next.downloadFont("Jua", { kind: "rank" });
  expect(update).toHaveBeenCalledWith({ rankStyle: { font: "Jua" } });
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

test("太さは 1 回だけ読み、読めなければ覚えずに読み直す", async () => {
  const read = vi.mocked(fontWeights);
  read.mockReset();
  const fresh = freshPanel();
  let fail!: (reason: Error) => void;
  read.mockReturnValueOnce(new Promise((_, reject) => (fail = reject)));
  const first = fresh.loadWeights("Jua");
  void fresh.loadWeights("Jua");
  expect(read).toHaveBeenCalledTimes(1);
  fail(new Error("offline"));
  await first;
  expect(fresh.weights.has("Jua")).toBe(false);

  read.mockResolvedValueOnce([400]);
  await fresh.loadWeights("Jua");
  await fresh.loadWeights("Jua");
  expect(fresh.weights.get("Jua")).toEqual([400]);
  expect(read).toHaveBeenCalledTimes(2);
});

test.each([
  ["chrome", "missing", "missing", "missing", []],
  ["gptTranscribe", "missing", "missing", "missing", ["openai"]],
  ["gptTranscribe", "saved", "missing", "missing", []],
  ["gptTranscribe", "missing", "saved", "missing", []],
  ["grokVoiceTranscribe", "missing", "missing", "missing", ["xai"]],
  ["grokVoiceTranscribe", "missing", "saved", "missing", []],
  ["grokVoiceTranscribe", "missing", "missing", "saved", []],
] as const)(
  "キーの案内: 確定の文字起こし %s、OpenAI %s、ギフトコード %s、xAI %s → %j",
  (transcription, openai, gift, xai, missing) => {
    const fresh = freshPanel();
    fresh.view = {
      settings: { engines: { ...CHROME, transcription } },
      keys: { openai: { state: openai }, gift: { state: gift }, xai: { state: xai } },
    } as unknown as SettingsView;
    expect(fresh.missingKeys).toEqual(missing);
    expect(fresh.needsKey).toBe(missing.length > 0);
  },
);

test("Grok と Terra なら、xAI と OpenAI の鍵が要る", () => {
  const fresh = freshPanel();
  fresh.view = {
    settings: {
      engines: { ...CHROME, transcription: "grokVoiceTranscribe", translation: "gpt61Terra" },
    },
    keys: { openai: { state: "missing" }, gift: { state: "missing" }, xai: { state: "missing" } },
  } as unknown as SettingsView;
  expect(fresh.missingKeys).toEqual(["openai", "xai"]);
});

test("ギフトコードで払ってもらうのは、自分のキーがない会社の分", () => {
  const fresh = freshPanel();
  fresh.view = {
    settings: {
      engines: { ...CHROME, transcription: "grokVoiceTranscribe", translation: "gpt61Terra" },
    },
    keys: { openai: { state: "saved" }, gift: { state: "saved" }, xai: { state: "missing" } },
  } as unknown as SettingsView;
  expect(fresh.giftPaid).toEqual(["xai"]);
});

test("中継のない会社（TypeSafe AI）は、コードがあっても自分のキーが要る", () => {
  const fresh = freshPanel();
  const keys = (typesafe: string) => ({
    openai: { state: "missing" },
    gift: { state: "missing" },
    beta: { state: "saved" },
    xai: { state: "missing" },
    qwen: { state: "missing" },
    openrouter: { state: "missing" },
    typesafe: { state: typesafe },
  });
  // 用語は Jev（中継なし）、翻訳は Qwen（ベータのコードで中継する。中継するかは、始めるときに中核が確かめる）
  const engines = { ...CHROME, terms: "jev", translation: "qwenMtFlash" };
  fresh.view = { settings: { engines }, keys: keys("missing") } as unknown as SettingsView;
  expect(fresh.missingKeys).toEqual(["typesafe"]);
  expect(fresh.giftPaid).toEqual(["qwen"]);
  fresh.view = { settings: { engines }, keys: keys("saved") } as unknown as SettingsView;
  expect(fresh.missingKeys).toEqual([]);
});

test("ベータのモードは、ベータのコードがあれば鍵が要らない（人からもらったギフトコードでは足りない）", () => {
  const fresh = freshPanel();
  const keys = (gift: string, beta: string) => ({
    openai: { state: "missing" },
    gift: { state: gift },
    beta: { state: beta },
    xai: { state: "missing" },
    qwen: { state: "missing" },
    openrouter: { state: "missing" },
    typesafe: { state: "missing" },
  });
  const view = (gift: string, beta: string) =>
    ({ settings: { engines: BETA }, keys: keys(gift, beta) }) as unknown as SettingsView;
  fresh.view = view("missing", "missing");
  expect(fresh.missingKeys).toEqual(["qwen", "openrouter"]);
  fresh.view = view("saved", "missing");
  expect(fresh.missingKeys).toEqual(["qwen", "openrouter"]);
  fresh.view = view("saved", "saved");
  expect(fresh.missingKeys).toEqual([]);
  expect(fresh.giftPaid).toEqual(["qwen", "openrouter"]);
});

test("設定が届く前はキーの案内を出さない", () => {
  expect(freshPanel().needsKey).toBe(false);
});

const betaKeys = (beta: string) => ({
  openai: { state: "missing" },
  gift: { state: "missing" },
  beta: { state: beta },
  xai: { state: "missing" },
  qwen: { state: "missing" },
  openrouter: { state: "missing" },
  typesafe: { state: "missing" },
});

test("ベータのコードが要るモードは、同意してからコードをもらって変える", async () => {
  const fresh = freshPanel();
  fresh.view = { settings: { engines: CHROME }, keys: betaKeys("missing") } as never;
  const update = vi.spyOn(fresh, "update").mockResolvedValue(true);
  const join = vi.spyOn(api, "joinBeta").mockResolvedValue({ state: "active" } as never);
  await fresh.chooseMode(BETA);
  expect(fresh.betaOffer).toEqual(BETA);
  expect(join).not.toHaveBeenCalled();
  expect(update).not.toHaveBeenCalled();
  expect(await fresh.acceptBeta()).toBe(true);
  expect(join).toHaveBeenCalledTimes(1);
  expect(update).toHaveBeenCalledWith({ engines: BETA });
  expect(fresh.betaOffer).toBeUndefined();
});

test("人からもらったギフトコードを持っていても、ベータのコードは別にもらう", async () => {
  const fresh = freshPanel();
  const keys = { ...betaKeys("missing"), gift: { state: "saved" } };
  fresh.view = { settings: { engines: CHROME }, keys } as never;
  vi.spyOn(fresh, "update").mockResolvedValue(true);
  await fresh.chooseMode(BETA);
  expect(fresh.betaOffer).toEqual(BETA);
});

test("ベータのコードをもらえなければ、モードを変えない", async () => {
  const fresh = freshPanel();
  fresh.view = { settings: { engines: CHROME }, keys: betaKeys("missing") } as never;
  const update = vi.spyOn(fresh, "update").mockResolvedValue(true);
  vi.spyOn(api, "joinBeta").mockResolvedValue({ error: "closed" } as never);
  await fresh.chooseMode(BETA);
  expect(await fresh.acceptBeta()).toBe(false);
  expect(update).not.toHaveBeenCalled();
  expect(fresh.betaOffer).toEqual(BETA);
});

test("ベータのコードがあるか、要らないモードは、聞かずに変える", async () => {
  const fresh = freshPanel();
  const update = vi.spyOn(fresh, "update").mockResolvedValue(true);
  fresh.view = { settings: { engines: CHROME }, keys: betaKeys("saved") } as never;
  await fresh.chooseMode(BETA);
  fresh.view = { settings: { engines: BETA }, keys: betaKeys("missing") } as never;
  await fresh.chooseMode(CHROME);
  expect(update.mock.calls).toEqual([[{ engines: BETA }], [{ engines: CHROME }]]);
  expect(fresh.betaOffer).toBeUndefined();
});
