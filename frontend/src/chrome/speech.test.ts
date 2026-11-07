import { afterEach, beforeEach, expect, test, vi } from "vitest";
import {
  chosenMic,
  DEFAULT_MIC,
  findMics,
  listen,
  micNotice,
  openMicrophone,
  rememberMic,
  supported,
} from "./speech";

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
  onTrouble: vi.fn(),
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
  vi.advanceTimersByTime(89_000);
  expect(FakeRecognizer.started).toBe(1);
  stuck.say("ヘイズが", false);
  vi.advanceTimersByTime(90_000);
  expect(stuck.abort).toHaveBeenCalled();
  expect(FakeRecognizer.started).toBe(2);
  expect(o.onText).toHaveBeenLastCalledWith("ヘイズが", true);
});

test("始めたまま進まない認識も作り直す", () => {
  listen("ja", { ...options(), silenceMs: undefined });
  vi.advanceTimersByTime(7999);
  expect(FakeRecognizer.started).toBe(1);
  vi.advanceTimersByTime(1);
  expect(FakeRecognizer.started).toBe(2);
});

test("動き始めたら、黙っていても 90 秒は作り直さない", () => {
  listen("ja", { ...options(), silenceMs: undefined });
  FakeRecognizer.last.onstart();
  vi.advanceTimersByTime(89_999);
  expect(FakeRecognizer.started).toBe(1);
  vi.advanceTimersByTime(1);
  expect(FakeRecognizer.started).toBe(2);
});

test("エラーがなければ、すぐ終わった認識も待たずに聞き直す", () => {
  listen("ja", options());
  FakeRecognizer.last.onend();
  vi.advanceTimersByTime(99);
  expect(FakeRecognizer.started).toBe(1);
  vi.advanceTimersByTime(1);
  expect(FakeRecognizer.started).toBe(2);
});

test("マイクのエラーでは止まらず、続いたら知らせて聞き直し、直ったら戻す", () => {
  const o = options();
  listen("ja", o);
  for (let i = 0; i < 3; i++) {
    FakeRecognizer.last.onerror({ error: "audio-capture" });
    FakeRecognizer.last.onend();
    vi.advanceTimersToNextTimer();
  }
  expect(o.onFatal).not.toHaveBeenCalled();
  expect(o.onTrouble).toHaveBeenCalledExactlyOnceWith("audio-capture");
  expect(FakeRecognizer.started).toBe(4);
  FakeRecognizer.last.say("ヘイズ", false);
  expect(o.onTrouble).toHaveBeenLastCalledWith();
});

test("知らない種類のエラーでも止まらない", () => {
  const o = options();
  listen("ja", o);
  FakeRecognizer.last.onerror({ error: "phrases-not-supported" });
  FakeRecognizer.last.onend();
  vi.advanceTimersByTime(2000);
  expect(o.onFatal).not.toHaveBeenCalled();
  expect(FakeRecognizer.started).toBe(2);
});

test.each(["not-allowed", "service-not-allowed", "language-not-supported"])(
  "%s では止まる",
  (error) => {
    const o = options();
    listen("ja", o);
    FakeRecognizer.last.onerror({ error });
    expect(o.onFatal).toHaveBeenCalledExactlyOnceWith(error);
  },
);

test("終わったトラックは渡さず、既定のマイクに戻す", () => {
  const ended = { kind: "audio", readyState: "live" } as { readyState: string };
  const onDefaultMic = vi.fn();
  listen("ja", {
    ...options(),
    track: ended as unknown as MediaStreamTrack,
    heardSound: () => false,
    onDefaultMic,
  });
  ended.readyState = "ended";
  FakeRecognizer.last.onend();
  vi.advanceTimersByTime(100);
  expect(onDefaultMic).toHaveBeenCalledOnce();
  expect(FakeRecognizer.last.start).toHaveBeenCalledWith();
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

test("トラックに音が来ていなければ、何回作り直しても既定のマイクに戻さない", () => {
  const onDefaultMic = vi.fn();
  listen("ja", { ...options(), track, heardSound: () => false, onDefaultMic });
  for (let i = 0; i < 5; i++) {
    FakeRecognizer.last.onend();
    vi.advanceTimersByTime(1000);
  }
  expect(onDefaultMic).not.toHaveBeenCalled();
  expect(FakeRecognizer.last.start).toHaveBeenCalledWith(track);
});

test("音が来ているのに認識が止まり続けたら既定のマイクに戻す", () => {
  const onDefaultMic = vi.fn();
  listen("ja", { ...options(), track, heardSound: () => true, onDefaultMic });
  vi.advanceTimersByTime(60_000);
  expect(onDefaultMic).toHaveBeenCalledOnce();
  expect(FakeRecognizer.last.start).toHaveBeenCalledWith();
});

test("無音で終わって再開するのを、止まったとみなさない", () => {
  listen("ja", options());
  const first = FakeRecognizer.last;
  for (let i = 0; i < 12; i++) {
    const recognizer = FakeRecognizer.last;
    recognizer.onstart();
    vi.advanceTimersByTime(10_000);
    recognizer.onerror({ error: "no-speech" });
    recognizer.onend();
    expect(recognizer.abort).not.toHaveBeenCalled();
  }
  expect(first.abort).not.toHaveBeenCalled();
  expect(FakeRecognizer.started).toBe(13);
});

test("途中の文は、変わらなかった前の部分も含めて渡す", () => {
  const o = options();
  listen("ja", o);
  FakeRecognizer.last.onresult({
    resultIndex: 1,
    results: [
      { isFinal: false, 0: { transcript: "ヘイズが" } },
      { isFinal: false, 0: { transcript: "強い" } },
    ],
  });
  expect(o.onText).toHaveBeenCalledExactlyOnceWith("ヘイズが強い", false);
});

test("確定した文は、次の結果で渡し直さない", () => {
  const o = options();
  listen("ja", o);
  FakeRecognizer.last.say("ヘイズ", true);
  FakeRecognizer.last.onresult({
    resultIndex: 1,
    results: [
      { isFinal: true, 0: { transcript: "ヘイズ" } },
      { isFinal: false, 0: { transcript: "強い" } },
    ],
  });
  expect(o.onText.mock.calls).toEqual([
    ["ヘイズ", true],
    ["強い", false],
  ]);
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
  expect(o.onTrouble).not.toHaveBeenCalled();
  for (let i = 0; i < 10; i++) failNetwork();
  expect(o.onFatal).not.toHaveBeenCalled();
  expect(o.onTrouble.mock.calls).toEqual([["network"]]);
  expect(FakeRecognizer.started).toBe(13);
});

test("文が届いたら回線が戻ったと知らせる", () => {
  const o = options();
  listen("ja", o);
  for (let i = 0; i < 3; i++) failNetwork();
  FakeRecognizer.last.say("ヘイズ", false);
  expect(o.onTrouble).toHaveBeenLastCalledWith();
});

test("黙っていても、通信のエラーが出なければ回線が戻ったと知らせる", () => {
  const o = options();
  listen("ja", o);
  for (let i = 0; i < 3; i++) failNetwork();
  vi.advanceTimersByTime(5000);
  expect(o.onTrouble).toHaveBeenLastCalledWith();
});

test("間に正常に終わった認識があれば、失敗を続けて数えない", () => {
  const o = options();
  listen("ja", o);
  for (let i = 0; i < 2; i++) failNetwork();
  FakeRecognizer.last.onend();
  vi.advanceTimersToNextTimer();
  for (let i = 0; i < 2; i++) failNetwork();
  expect(o.onTrouble).not.toHaveBeenCalled();
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

class FakeStream {
  stopped = false;
  track = {
    kind: "audio",
    label: "",
    stop: () => {
      this.stopped = true;
    },
  };
  getTracks = () => [this.track];
  getAudioTracks = () => [this.track];
}

/** `error` を渡すと、`refused` のマイク（省くとすべて）を開けない */
function fakeDevices(
  inputs: { deviceId: string; label: string }[],
  error?: string,
  refused?: string[],
) {
  const streams: { audio: unknown; stream: FakeStream }[] = [];
  const getUserMedia = vi.fn(async ({ audio }: { audio: unknown }) => {
    const id = (audio as { deviceId?: { exact: string } }).deviceId?.exact;
    if (error && id && (!refused || refused.includes(id))) throw new DOMException("", error);
    const stream = new FakeStream();
    streams.push({ audio, stream });
    return stream;
  });
  const enumerateDevices = vi.fn(async () => inputs.map((d) => ({ ...d, kind: "audioinput" })));
  vi.stubGlobal("navigator", {
    mediaDevices: { getUserMedia, enumerateDevices },
  });
  vi.stubGlobal(
    "AudioContext",
    class {
      state = "running";
      createAnalyser = () => ({ fftSize: 32 });
      createMediaStreamSource = () => ({ connect() {} });
      close = async () => {};
    },
  );
  return { streams, getUserMedia, enumerateDevices };
}

const WINDOWS_MICS = [
  { deviceId: "default", label: "既定 - マイク (Realtek Audio)" },
  { deviceId: "communications", label: "通信 - マイク (Realtek Audio)" },
  { deviceId: "r", label: "マイク (Realtek Audio)" },
  { deviceId: "u", label: "マイク (Yeti Stereo) (b58e:9e84)" },
];

const found = (name: string, inputs: { deviceId: string; label: string }[]) =>
  findMics(inputs as MediaDeviceInfo[], name).map((d) => d.deviceId);

test.each([
  ["マイク (Yeti Stereo)", "u"],
  ["  マイク  (Yeti Stereo) ", "u"],
  ["マイク (Realtek Audio)", "r"],
])("Windows のラベルから %s を見つける", (name, id) => {
  expect(found(name, WINDOWS_MICS)).toEqual([id]);
});

test.each([
  ["Headset", "Headset (Bluetooth)"],
  ["MacBook Pro Microphone", "MacBook Pro Microphone (Built-in)"],
  ["マイク (Logitech(R) USB)", "マイク (Logitech(R) USB) (046d:0825)"],
  ["マイク", "マイク（Virtual）"],
])("%s を、末尾の括弧を 1 つ除いて %s に当てる", (name, label) => {
  expect(found(name, [{ deviceId: "x", label }])).toEqual(["x"]);
});

test("括弧は 1 つしか除かない", () => {
  expect(found("Mic", [{ deviceId: "x", label: "Mic (USB) (0001:0001)" }])).toEqual([]);
});

test("全体が一致するマイクがあれば、末尾の括弧が違うだけのマイクには当てない", () => {
  const mics = [
    { deviceId: "virtual", label: "Mic (Virtual)" },
    { deviceId: "real", label: "Mic" },
  ];
  expect(found("Mic", mics)).toEqual(["real"]);
});

test("既定と通信の別名には当てない", () => {
  expect(found("マイク (Realtek Audio)", WINDOWS_MICS.slice(0, 2))).toEqual([]);
});

test.each([
  ["USB Mic", "USB Mic 2"],
  ["USB Mic 2", "USB Mic"],
  ["マイク (Realtek Audio)", "マイク (2- Realtek Audio)"],
  ["マイク (Realtek", "マイク (Realtek Audio)"],
])("%s を %s に当てない", (name, label) => {
  expect(found(name, [{ deviceId: "x", label }])).toEqual([]);
});

test("同じ名前が 2 つなら全体の一致を優先し、それでも残れば両方返す", () => {
  const twins = [
    { deviceId: "a", label: "マイク (USB) (0001:0001)" },
    { deviceId: "b", label: "マイク (USB)" },
  ];
  expect(found("マイク (USB)", twins)).toEqual(["b"]);
  const same = [
    { deviceId: "a", label: "マイク (USB) (0001:0001)" },
    { deviceId: "b", label: "マイク (USB) (0002:0002)" },
  ];
  expect(found("マイク (USB)", same)).toEqual(["a", "b"]);
});

test("ラベルが読めるなら、既定のマイクを開かずに選んだマイクを開く", async () => {
  const { streams, getUserMedia } = fakeDevices(WINDOWS_MICS);
  const microphone = await openMicrophone("マイク (Yeti Stereo)");
  expect(getUserMedia).toHaveBeenCalledExactlyOnceWith({
    audio: { deviceId: { exact: "u" } },
  });
  expect(microphone.trouble).toBeUndefined();
  expect(microphone.track).toBe(streams[0]!.stream.track);
  expect(microphone.opened?.deviceId).toBe("u");
  microphone.release();
  expect(streams[0]!.stream.stopped).toBe(true);
});

test("ラベルが読めないときだけ既定のマイクを開き、選んだマイクを開く前に閉じる", async () => {
  const fake = fakeDevices(WINDOWS_MICS);
  fake.enumerateDevices.mockResolvedValueOnce([{ deviceId: "", label: "", kind: "audioinput" }]);
  const closedFirst: boolean[] = [];
  const open = fake.getUserMedia.getMockImplementation()!;
  fake.getUserMedia.mockImplementation(async (request) => {
    if (request.audio !== true) closedFirst.push(fake.streams[0]!.stream.stopped);
    return open(request);
  });
  const microphone = await openMicrophone("マイク (Yeti Stereo)");
  expect(closedFirst).toEqual([true]);
  expect(microphone.track).toBeDefined();
});

test("マイクが見つからなければ、既定のマイクを開いて理由を返す", async () => {
  const { streams } = fakeDevices(WINDOWS_MICS);
  const log = vi.fn();
  const microphone = await openMicrophone("抜いたマイク", log);
  expect(microphone.trouble).toEqual({ name: "抜いたマイク", problem: { type: "missing" } });
  expect(microphone.track).toBeUndefined();
  expect(streams.map((s) => s.audio)).toEqual([true]);
  expect(micNotice(microphone.trouble!)).toContain("Chrome に『抜いたマイク』が見つかりません");
  expect(log.mock.calls.flat()[0]).toContain("マイク (Yeti Stereo) (b58e:9e84)");
});

test("マイクを開けなければ、エラー名を返して既定のマイクを開く", async () => {
  const { streams } = fakeDevices(WINDOWS_MICS, "NotReadableError");
  const log = vi.fn();
  const microphone = await openMicrophone("マイク (Yeti Stereo)", log);
  expect(microphone.trouble).toEqual({
    name: "マイク (Yeti Stereo) (b58e:9e84)",
    problem: { type: "unopenable", error: "NotReadableError" },
  });
  expect(streams).toHaveLength(1);
  expect(micNotice(microphone.trouble!)).toContain(
    "『マイク (Yeti Stereo) (b58e:9e84)』を開けません（NotReadableError）。Chrome の既定のマイクを使っています",
  );
  expect(log).toHaveBeenCalledWith(
    "『マイク (Yeti Stereo) (b58e:9e84)』の getUserMedia のエラー: NotReadableError",
  );
});

test("覚えたマイクを開けなければ、その名前を返してアプリのマイクを開く", async () => {
  const { streams } = fakeDevices(WINDOWS_MICS, "NotReadableError", ["r"]);
  const microphone = await openMicrophone("マイク (Yeti Stereo)", undefined, {
    deviceId: "r",
    label: "マイク (Realtek Audio)",
  });
  expect(microphone.opened?.deviceId).toBe("u");
  expect(microphone.track).toBe(streams[0]!.stream.track);
  expect(microphone.trouble?.name).toBe("マイク (Realtek Audio)");
  expect(micNotice(microphone.trouble!, microphone.opened?.label)).toBe(
    "『マイク (Realtek Audio)』を開けません（NotReadableError）。『マイク (Yeti Stereo) (b58e:9e84)』を使っています",
  );
});

test("同じ名前のマイクの 1 つ目を開けなければ、2 つ目を開く", async () => {
  fakeDevices(
    [
      { deviceId: "a", label: "A (0001:0001)" },
      { deviceId: "b", label: "A (0002:0002)" },
    ],
    "NotReadableError",
    ["a"],
  );
  const microphone = await openMicrophone("A");
  expect(microphone.opened?.deviceId).toBe("b");
});

test("どのマイクも開けなければ、アプリのマイクの理由を返して既定のマイクを開く", async () => {
  const { streams } = fakeDevices(WINDOWS_MICS, "NotReadableError");
  const microphone = await openMicrophone("マイク (Yeti Stereo)", undefined, {
    deviceId: "r",
    label: "マイク (Realtek Audio)",
  });
  expect(microphone.trouble?.name).toBe("マイク (Yeti Stereo) (b58e:9e84)");
  expect(microphone.chosen?.deviceId).toBe("r");
  expect(microphone.opened).toBeUndefined();
  expect(streams.map((s) => s.audio)).toEqual([true]);
});

test("既定のマイクを選んであれば、アプリのマイクが見つかっても既定のマイクを開く", async () => {
  const { streams } = fakeDevices(WINDOWS_MICS);
  const microphone = await openMicrophone("マイク (Yeti Stereo)", undefined, DEFAULT_MIC);
  expect(streams.map((s) => s.audio)).toEqual([true]);
  expect(microphone.track).toBeUndefined();
  expect(microphone.trouble).toBeUndefined();
  expect(microphone.chosen).toEqual(DEFAULT_MIC);
  expect(microphone.choices).toHaveLength(2);
});

test("名前がなければ既定のマイクを開き、一覧は見ない", async () => {
  const { enumerateDevices, getUserMedia } = fakeDevices(WINDOWS_MICS);
  const microphone = await openMicrophone(null);
  expect(getUserMedia).toHaveBeenCalledExactlyOnceWith({ audio: true });
  expect(enumerateDevices).not.toHaveBeenCalled();
  expect(microphone.trouble).toBeUndefined();
});

test("既定に戻すと選んだマイクを閉じて既定のマイクを開き、止めたら閉じる", async () => {
  const { streams } = fakeDevices(WINDOWS_MICS);
  const microphone = await openMicrophone("マイク (Yeti Stereo)");
  await microphone.useDefault();
  expect(streams[0]!.stream.stopped).toBe(true);
  expect(streams[1]!.audio).toBe(true);
  expect(streams[1]!.stream.stopped).toBe(false);
  microphone.release();
  expect(streams[1]!.stream.stopped).toBe(true);
});

test("既定を開いている間に止めたら、開いた既定のマイクも閉じる", async () => {
  const { streams } = fakeDevices(WINDOWS_MICS);
  const microphone = await openMicrophone("マイク (Yeti Stereo)");
  const going = microphone.useDefault();
  microphone.release();
  await going;
  expect(streams.map((s) => s.stream.stopped)).toEqual([true, true]);
});

test("音が認識に届かないまま既定に戻す案内と、start(track) の記録を残す", () => {
  const onMicLog = vi.fn();
  const onDefaultMic = vi.fn();
  listen("ja", { ...options(), track, onDefaultMic, onMicLog });
  FakeRecognizer.last.onstart();
  FakeRecognizer.last.onend();
  vi.advanceTimersByTime(1000);
  FakeRecognizer.last.onstart();
  FakeRecognizer.last.onend();
  vi.advanceTimersByTime(1000);
  expect(onDefaultMic).toHaveBeenCalledOnce();
  expect(onMicLog.mock.calls.flat()).toEqual([
    "start(track): 認識が動き始めました",
    "音が来ているのに結果がありません（1 回目）",
    "音が来ているのに結果がありません（2 回目）",
    '既定のマイクに戻します: {"type":"silent"}',
  ]);
  expect(micNotice({ name: "A", problem: { type: "silent" } })).toContain(
    "『A』の音が認識に届きません",
  );
});

test("見つからないときは選べるマイクを返し、別名は入れない", async () => {
  fakeDevices(WINDOWS_MICS);
  const microphone = await openMicrophone("抜いたマイク");
  expect(microphone.choices.map((d) => d.deviceId)).toEqual(["r", "u"]);
});

test("覚えた選択を優先し、ID が変わってもラベルで見つける", async () => {
  const { getUserMedia } = fakeDevices(WINDOWS_MICS);
  const byId = await openMicrophone("A", undefined, { deviceId: "u", label: "別の名前" });
  expect(byId.chosen?.deviceId).toBe("u");
  expect(byId.trouble).toBeUndefined();
  const byLabel = await openMicrophone("A", undefined, {
    deviceId: "古い ID",
    label: "マイク (Realtek Audio)",
  });
  expect(byLabel.chosen?.deviceId).toBe("r");
  expect(getUserMedia).toHaveBeenLastCalledWith({ audio: { deviceId: { exact: "r" } } });
});

test("覚えた選択が見つからなければ理由と選択欄に戻る", async () => {
  fakeDevices(WINDOWS_MICS);
  const microphone = await openMicrophone("A", undefined, { deviceId: "z", label: "ない" });
  expect(microphone.chosen).toBeUndefined();
  expect(microphone.trouble).toEqual({ name: "ない", problem: { type: "missing" } });
  expect(microphone.choices).toHaveLength(2);
});

test("覚えた選択が見つからなくてもアプリのマイクを開き、理由を返す", async () => {
  fakeDevices(WINDOWS_MICS);
  const microphone = await openMicrophone("マイク (Yeti Stereo)", undefined, {
    deviceId: "z",
    label: "ない",
  });
  expect(microphone.opened?.deviceId).toBe("u");
  expect(micNotice(microphone.trouble!, microphone.opened?.label)).toBe(
    "Chrome に『ない』が見つかりません。『マイク (Yeti Stereo) (b58e:9e84)』を使っています",
  );
});

test("start(track) のない Chrome では理由を返し、既定のマイクを使う", async () => {
  const { enumerateDevices } = fakeDevices(WINDOWS_MICS);
  vi.stubGlobal("navigator", {
    ...navigator,
    userAgentData: { brands: [{ brand: "Google Chrome", version: "134" }] },
  });
  const microphone = await openMicrophone("A");
  expect(microphone.trouble?.problem).toEqual({ type: "oldChrome", version: 134 });
  expect(enumerateDevices).not.toHaveBeenCalled();
  expect(micNotice(microphone.trouble!)).toContain("Chrome 135 以降");
});

test("切れたマイクの理由を知らせる", () => {
  const ended = { kind: "audio", readyState: "live" } as { readyState: string };
  const onDefaultMic = vi.fn();
  listen("ja", { ...options(), track: ended as unknown as MediaStreamTrack, onDefaultMic });
  ended.readyState = "ended";
  FakeRecognizer.last.onend();
  vi.advanceTimersByTime(100);
  expect(onDefaultMic).toHaveBeenCalledWith({ type: "ended" });
  expect(micNotice({ name: "A", problem: { type: "ended" } })).toContain("『A』が切れました");
});

const fakeStorage = (saved: string | null) => {
  const storage = {
    saved,
    getItem: () => storage.saved,
    setItem: (_key: string, value: string) => (storage.saved = value),
  };
  vi.stubGlobal("localStorage", storage);
};

test("選んだマイクを覚え、アプリのマイクが同じ間だけ返す", () => {
  fakeStorage(null);
  expect(chosenMic("A")).toBeUndefined();
  rememberMic({ deviceId: "u", label: "ラベル" }, "A");
  expect(chosenMic("A")).toEqual({ deviceId: "u", label: "ラベル" });
  expect(chosenMic("B")).toBeUndefined();
});

test("既定のマイクの選択も覚える", () => {
  fakeStorage(null);
  rememberMic(DEFAULT_MIC, "A");
  expect(chosenMic("A")).toEqual(DEFAULT_MIC);
});

test.each(["{壊れた", "1", '"文字"', "[]", '{"name":"A"}', '{"name":"A","deviceId":1,"label":""}'])(
  "覚えた選択 %s は読めないので、選択なしとして続ける",
  (saved) => {
    fakeStorage(saved);
    expect(chosenMic("A")).toBeUndefined();
  },
);

test("localStorage が使えなくても、選択なしとして続ける", () => {
  vi.stubGlobal("localStorage", {
    getItem() {
      throw new DOMException("", "SecurityError");
    },
  });
  expect(chosenMic("A")).toBeUndefined();
});
