import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { listen, supported } from "./speech";

class FakeRecognizer {
  static last: FakeRecognizer;
  static started = 0;
  lang = "";
  continuous = false;
  interimResults = false;
  onresult: (event: unknown) => void = () => {};
  onerror: (event: unknown) => void = () => {};
  onend = () => {};
  stop = vi.fn(() => this.onend());

  constructor() {
    FakeRecognizer.last = this;
  }
  start() {
    FakeRecognizer.started++;
  }
  say(transcript: string, isFinal: boolean) {
    this.onresult({ resultIndex: 0, results: [{ isFinal, 0: { transcript } }] });
  }
}

const options = () => ({ silenceMs: 750, onText: vi.fn(), onFatal: vi.fn() });

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("window", { SpeechRecognition: FakeRecognizer });
  FakeRecognizer.started = 0;
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

test("認識の形を設定する", () => {
  listen("ja", options());
  const { lang, continuous, interimResults } = FakeRecognizer.last;
  expect({ lang, continuous, interimResults }).toEqual({
    lang: "ja-JP",
    continuous: true,
    interimResults: true,
  });
});

test("途中と確定の文を渡す", () => {
  const o = options();
  listen("ja", o);
  FakeRecognizer.last.say(" ヘイズ", false);
  FakeRecognizer.last.say(" ヘイズが強い", true);
  expect(o.onText.mock.calls).toEqual([
    ["ヘイズ", false],
    ["ヘイズが強い", true],
  ]);
});

test("文字が増えなければ区切り、確定しなかった文を確定して聞き直す", () => {
  const o = options();
  listen("ja", o);
  FakeRecognizer.last.say("ヘイズ", false);
  vi.advanceTimersByTime(749);
  expect(FakeRecognizer.last.stop).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1000);
  expect(o.onText).toHaveBeenLastCalledWith("ヘイズ", true);
  expect(FakeRecognizer.started).toBe(2);
});

test("切れたら始め直す", () => {
  listen("ja", options());
  FakeRecognizer.last.onerror({ error: "no-speech" });
  FakeRecognizer.last.onend();
  vi.advanceTimersByTime(1000);
  expect(FakeRecognizer.started).toBe(2);
});

test("止めたら途中の文を捨て、始め直さない", () => {
  const o = options();
  const stop = listen("ja", o);
  FakeRecognizer.last.say("ヘイズ", false);
  stop();
  vi.advanceTimersByTime(5000);
  expect(o.onText).toHaveBeenLastCalledWith("", true);
  expect(FakeRecognizer.started).toBe(1);
});

test("マイクを使えなければ知らせて止まる", () => {
  const o = options();
  listen("ja", o);
  FakeRecognizer.last.onerror({ error: "not-allowed" });
  FakeRecognizer.last.onend();
  vi.advanceTimersByTime(5000);
  expect(o.onFatal).toHaveBeenCalledWith("not-allowed");
  expect(FakeRecognizer.started).toBe(1);
});

test("続けて通信できなければ止まる", () => {
  const o = options();
  listen("ja", o);
  for (let i = 0; i < 5; i++) FakeRecognizer.last.onerror({ error: "network" });
  expect(o.onFatal).toHaveBeenCalledWith("network");
});

test("Google Chrome のほかは使えない", () => {
  const withBrand = (brand: string) =>
    vi.stubGlobal("navigator", { userAgentData: { brands: [{ brand }] } });
  withBrand("Google Chrome");
  expect(supported()).toBe(true);
  withBrand("Microsoft Edge");
  expect(supported()).toBe(false);
  vi.stubGlobal("navigator", {});
  expect(supported()).toBe(false);
});
