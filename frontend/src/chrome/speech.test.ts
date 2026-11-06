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
  onstart = () => {};
  onaudiostart = () => {};
  stop = vi.fn(() => this.onend());
  abort = vi.fn();

  constructor() {
    FakeRecognizer.last = this;
  }
  start = vi.fn((_track?: unknown) => {
    FakeRecognizer.started++;
  });
  say(transcript: string, isFinal: boolean) {
    this.onresult({ resultIndex: 0, results: [{ isFinal, 0: { transcript } }] });
  }
}

const options = () => ({
  silenceMs: 750,
  onText: vi.fn(),
  onFatal: vi.fn(),
  onOffline: vi.fn(),
});

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
  FakeRecognizer.last.say("ヘイズ", false);
  FakeRecognizer.last.say("ヘイズが強い", true);
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

test("区切りの時間を省くと、文字が増えなくても止めない", () => {
  listen("ja", { ...options(), silenceMs: undefined });
  FakeRecognizer.last.say("ヘイズ", false);
  vi.advanceTimersByTime(60_000);
  expect(FakeRecognizer.last.stop).not.toHaveBeenCalled();
});

test("区切ったら途中の文を確定せず、聞き直すまでの結果を使わない", () => {
  const o = options();
  const { cut } = listen("ja", { ...o, silenceMs: undefined });
  const before = FakeRecognizer.last;
  before.say("ヘイズ", false);
  o.onText.mockClear();
  before.stop.mockImplementationOnce(() => undefined);
  cut();
  before.say("遅れた文", false);
  before.onend();
  vi.advanceTimersByTime(1000);
  expect(o.onText).not.toHaveBeenCalled();
  expect(FakeRecognizer.started).toBe(2);
});

test("再開のたびに作り直し、古いインスタンスの遅れた知らせを使わない", () => {
  const o = options();
  listen("ja", o);
  const old = FakeRecognizer.last;
  old.onend();
  vi.advanceTimersByTime(1000);
  expect(FakeRecognizer.last).not.toBe(old);

  o.onText.mockClear();
  old.say("遅れた文", false);
  old.onerror({ error: "not-allowed" });
  old.onend();
  vi.advanceTimersByTime(5000);
  expect(o.onText).not.toHaveBeenCalled();
  expect(o.onFatal).not.toHaveBeenCalled();
  expect(FakeRecognizer.started).toBe(2);
});

test("なにも届かなくなった認識を作り直す", () => {
  const o = options();
  listen("ja", { ...o, silenceMs: undefined });
  const stuck = FakeRecognizer.last;
  stuck.say("ヘイズ", false);
  vi.advanceTimersByTime(29_000);
  expect(FakeRecognizer.started).toBe(1);
  stuck.say("ヘイズが", false);
  vi.advanceTimersByTime(30_000);
  expect(stuck.abort).toHaveBeenCalled();
  expect(FakeRecognizer.started).toBe(2);
  expect(o.onText).toHaveBeenLastCalledWith("ヘイズが", true);
});

test("始めたまま進まない認識も作り直す", () => {
  listen("ja", { ...options(), silenceMs: undefined });
  vi.advanceTimersByTime(30_000);
  expect(FakeRecognizer.started).toBe(2);
});

test("聞き直しを待っている間は作り直さない", () => {
  listen("ja", options());
  FakeRecognizer.last.onerror({ error: "no-speech" });
  FakeRecognizer.last.onend();
  vi.advanceTimersByTime(1000);
  expect(FakeRecognizer.started).toBe(2);
});

test("止めたら作り直さない", () => {
  const { stop } = listen("ja", options());
  stop();
  vi.advanceTimersByTime(60_000);
  expect(FakeRecognizer.started).toBe(1);
});

const track = { kind: "audio" } as unknown as MediaStreamTrack;

test("マイクのトラックを認識に渡す", () => {
  listen("ja", { ...options(), track });
  expect(FakeRecognizer.last.start).toHaveBeenCalledWith(track);
});

test("トラックを受け付けなければ既定のマイクに戻す", () => {
  const onDefaultMic = vi.fn();
  class Refusing extends FakeRecognizer {
    override start = vi.fn((given?: unknown) => {
      if (given) throw new TypeError("no track");
      FakeRecognizer.started++;
    });
  }
  vi.stubGlobal("window", { SpeechRecognition: Refusing });
  listen("ja", { ...options(), track, onDefaultMic });
  expect(onDefaultMic).toHaveBeenCalled();
  expect(FakeRecognizer.started).toBe(1);
});

test("トラックで結果が来ないまま 2 回作り直したら既定のマイクに戻す", () => {
  const onDefaultMic = vi.fn();
  listen("ja", { ...options(), track, onDefaultMic });
  FakeRecognizer.last.onend();
  vi.advanceTimersByTime(1000);
  expect(onDefaultMic).not.toHaveBeenCalled();
  FakeRecognizer.last.onend();
  vi.advanceTimersByTime(1000);
  expect(onDefaultMic).toHaveBeenCalledOnce();
  expect(FakeRecognizer.last.start).toHaveBeenCalledWith();
});

test("トラックで結果が届いたら、黙っていても既定のマイクに戻さない", () => {
  const onDefaultMic = vi.fn();
  listen("ja", { ...options(), track, onDefaultMic });
  FakeRecognizer.last.say("ヘイズ", true);
  for (let i = 0; i < 5; i++) {
    FakeRecognizer.last.onend();
    vi.advanceTimersByTime(1000);
  }
  expect(onDefaultMic).not.toHaveBeenCalled();
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
  const { stop } = listen("ja", o);
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

/** 通信のエラーで終わる認識を、次の認識が始まるまで進める */
function failNetwork() {
  FakeRecognizer.last.onerror({ error: "network" });
  FakeRecognizer.last.onend();
  vi.advanceTimersToNextTimer();
}

test("通信できない間は止まらず、知らせて聞き直し続ける", () => {
  const o = options();
  listen("ja", o);
  for (let i = 0; i < 2; i++) failNetwork();
  expect(o.onOffline).not.toHaveBeenCalled();
  for (let i = 0; i < 10; i++) failNetwork();
  expect(o.onFatal).not.toHaveBeenCalled();
  expect(o.onOffline.mock.calls).toEqual([[true]]);
  expect(FakeRecognizer.started).toBe(13);
});

test("文が届いたら回線が戻ったと知らせる", () => {
  const o = options();
  listen("ja", o);
  for (let i = 0; i < 3; i++) failNetwork();
  FakeRecognizer.last.say("ヘイズ", false);
  expect(o.onOffline).toHaveBeenLastCalledWith(false);
});

test("黙っていても、通信のエラーが出なければ回線が戻ったと知らせる", () => {
  const o = options();
  listen("ja", o);
  for (let i = 0; i < 3; i++) failNetwork();
  vi.advanceTimersByTime(5000);
  expect(o.onOffline).toHaveBeenLastCalledWith(false);
});

test("間に正常に終わった認識があれば、失敗を続けて数えない", () => {
  const o = options();
  listen("ja", o);
  for (let i = 0; i < 2; i++) failNetwork();
  FakeRecognizer.last.onend();
  vi.advanceTimersToNextTimer();
  for (let i = 0; i < 2; i++) failNetwork();
  expect(o.onOffline).not.toHaveBeenCalled();
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
