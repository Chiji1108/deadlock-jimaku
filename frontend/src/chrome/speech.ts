// Chrome の音声認識と翻訳（Web Speech API と Translator API）。音声は Google に送られ、翻訳は端末の中で動く

type Availability = "unavailable" | "downloadable" | "downloading" | "available";

interface Alternative {
  transcript: string;
}
interface Result {
  isFinal: boolean;
  0: Alternative;
}
interface ResultEvent {
  resultIndex: number;
  results: { length: number; [index: number]: Result };
}
interface Recognizer {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  onstart: () => void;
  onaudiostart: () => void;
  onresult: (event: ResultEvent) => void;
  onerror: (event: { error: string }) => void;
  onend: () => void;
  start(track?: MediaStreamTrack): void;
  stop(): void;
  abort(): void;
}
interface RecognizerClass {
  new (): Recognizer;
}
interface Pair {
  sourceLanguage: string;
  targetLanguage: string;
}
export interface Translator {
  translate(text: string): Promise<string>;
}
interface Monitor {
  addEventListener(type: "downloadprogress", on: (event: { loaded: number }) => void): void;
}
interface TranslatorClass {
  availability(pair: Pair): Promise<Availability>;
  create(options: Pair & { monitor?(monitor: Monitor): void }): Promise<Translator>;
}

/** 続けても直らないエラー。それ以外は認識を始め直す */
const FATAL = new Set(["not-allowed", "service-not-allowed", "language-not-supported"]);
/** ふだんの終わり方。問題として数えない */
const EXPECTED = new Set(["no-speech", "aborted"]);
/** すぐ終わり続けるときに、聞き直しを連打しない。短い発話のあとの声を落とさないよう短くする */
const MIN_RESTART_MS = 100;
/** エラーが続くときの聞き直しの間隔の元。翻訳のモデルの作り直しの待ちにも使う */
const RETRY_MS = 1000;
const CREATE_ATTEMPTS = 3;
/** エラーが続く間の始め直しの間隔の上限。直ればこの時間のうちに再開する */
const MAX_RETRY_MS = 15_000;
/** 続けて失敗したら知らせる */
const TROUBLE_FAILURES = 3;
/** 始めてから `onstart` が来るまでの時間。これを過ぎたら止まったとみなして作り直す */
const START_MS = 8000;
/** 動き始めてから、この時間なにも届かなければ止まったとみなして作り直す。
 * `continuous` の認識は黙っている間なにも届かないことがあるので長くする */
const STALL_MS = 90_000;
/** マイクのトラックに音が来ているのに結果が来ないまま、この回数作り直したら既定のマイクに戻す。
 * 受け付けても黙って動かない環境がある。黙っているだけなら数えない */
const SILENT_TRACK_LIMIT = 2;
/** トラックの音を見る間隔と、音が来ているとみなす振幅（-40 dBFS。部屋の雑音は数えない） */
const SOUND_CHECK_MS = 250;
const SOUND_LEVEL = 0.01;
/** 通信のエラーが出ずにこの時間続いたら、回線が戻ったとみなす（黙っていると結果が届かない） */
const STABLE_MS = 5000;
/** `start(track)` が使える Chrome の版 */
const TRACK_CHROME = 135;
/** アプリが CDP でマイクを許可するまで待つ時間 */
const GRANT_MS = 5000;
/** 「既定」「通信」のデバイス ID。本物のマイクの別名で、接頭辞は Chrome の言語で変わるので ID で除く */
const MIC_ALIASES = new Set(["default", "communications"]);

const recognition = () => {
  const { SpeechRecognition, webkitSpeechRecognition } = window as unknown as {
    SpeechRecognition?: RecognizerClass;
    webkitSpeechRecognition?: RecognizerClass;
  };
  return SpeechRecognition ?? webkitSpeechRecognition;
};

const translatorApi = () => (window as unknown as { Translator?: TranslatorClass }).Translator;

const chromeBrand = () => {
  const { userAgentData } = navigator as unknown as {
    userAgentData?: { brands: { brand: string; version: string }[] };
  };
  return userAgentData?.brands.find((b) => b.brand === "Google Chrome");
};

/** Google Chrome か。Edge や WebView2 は音声の送り先が違い、認識も失敗することがある */
export function supported() {
  return !!recognition() && !!chromeBrand();
}

/** `ja` を認識に渡す形（`ja-JP`）にする */
const speechTag = (lang: string) => {
  const { language, region } = new Intl.Locale(lang).maximize();
  return `${language}-${region}`;
};

const pairs = (lang: string, targets: string[]) =>
  targets.map((targetLanguage) => ({ sourceLanguage: lang, targetLanguage }));

/** 使えるか。downloadable はダウンロードすれば使える（クリックの中で `prepare` する）。
 * `targets` は訳せる言語だけに絞る（訳せない言語があっても、ほかの言語で字幕は出せる） */
export async function availability(lang: string, targets: string[]) {
  const api = translatorApi();
  if (!recognition() || !api) {
    return { state: "unavailable" as Availability, targets: [] as string[] };
  }
  const translation = await Promise.all(pairs(lang, targets).map((pair) => api.availability(pair)));
  const ready = translation.every((a) => a === "available" || a === "unavailable");
  return {
    state: (ready ? "available" : "downloadable") as Availability,
    targets: targets.filter((_, i) => translation[i] !== "unavailable"),
  };
}

/** 初回はモデルの準備中に失敗することがあるので、少し待って作り直す */
async function create(
  pair: Pair,
  onProgress: (loaded: number) => void,
  attempts = CREATE_ATTEMPTS,
): Promise<Translator> {
  const monitor = (m: Monitor) =>
    m.addEventListener("downloadprogress", ({ loaded }) => onProgress(loaded));
  try {
    return await translatorApi()!.create({ ...pair, monitor });
  } catch (e) {
    if (attempts <= 1) throw e;
    await new Promise((resolve) => setTimeout(resolve, RETRY_MS));
    return create(pair, onProgress, attempts - 1);
  }
}

/** 翻訳モデルを入れる。ダウンロードにはクリックが要る。`onProgress` は 0〜1 */
export async function prepare(
  lang: string,
  targets: string[],
  onProgress: (loaded: number) => void,
): Promise<Map<string, Translator>> {
  const created = new Map<string, Translator>();
  for (const pair of pairs(lang, targets)) {
    created.set(pair.targetLanguage, await create(pair, onProgress));
  }
  return created;
}

/** 選んだマイクを使えない理由 */
export type MicProblem =
  | { type: "missing" }
  | { type: "unopenable"; error: string }
  | { type: "silent" }
  | { type: "ended" }
  | { type: "oldChrome"; version: number };

const micReason = (name: string, problem: MicProblem) => {
  switch (problem.type) {
    case "missing":
      return `Chrome に『${name}』が見つかりません`;
    case "unopenable":
      return `『${name}』を開けません（${problem.error}）`;
    case "silent":
      return `『${name}』の音が認識に届きません`;
    case "ended":
      return `『${name}』が切れました`;
    case "oldChrome":
      return `この Chrome（${problem.version}）は『${name}』を認識に渡せません。Chrome ${TRACK_CHROME} 以降にしてください`;
  }
};

/** 理由と、既定のマイクで続けていることを知らせる文。Chrome の既定は OS の既定のマイク */
export function micNotice(name: string, problem: MicProblem) {
  return `${micReason(name, problem)}。Chrome の既定のマイクを使っています。OS の既定のマイクを『${name}』にすると揃います`;
}

const tidy = (text: string) => text.normalize("NFKC").trim().replace(/\s+/g, " ");

/** Chrome がラベルの末尾に足すものを除く。Windows は (045e:0810) か (Bluetooth)、Mac は (Built-in) など */
const withoutSuffix = (label: string) =>
  label.replace(/\s*\((?:[0-9a-f]{4}:[0-9a-f]{4}|Bluetooth|Built-in)\)$/, "");

const realMics = (inputs: MediaDeviceInfo[]) =>
  inputs.filter((d) => d.deviceId && !MIC_ALIASES.has(d.deviceId));

/** アプリのマイクの名前に当たるデバイスを返す。部分一致は別のマイクに当たるので、全体を比べる。
 * 同じ名前が複数なら見分けられない（アプリも先頭のものを開く） */
export function findMics(inputs: MediaDeviceInfo[], name: string) {
  const mics = realMics(inputs);
  const wanted = tidy(name);
  const exact = mics.filter((d) => tidy(d.label) === wanted);
  return exact.length ? exact : mics.filter((d) => tidy(withoutSuffix(d.label)) === wanted);
}

/** アプリの許可が届くまで待つ。アプリの外で開いたページなら、待ったあと `getUserMedia` で許可を聞く */
async function micPermission() {
  const status = await navigator.permissions.query({ name: "microphone" as PermissionName });
  if (status.state !== "prompt") return;
  await new Promise<void>((resolve) => {
    setTimeout(resolve, GRANT_MS);
    status.addEventListener("change", () => resolve(), { once: true });
  });
}

const openAudio = (audio: MediaStreamConstraints["audio"]) =>
  navigator.mediaDevices.getUserMedia({ audio });
const stopStream = (stream: MediaStream) => stream.getTracks().forEach((track) => track.stop());
const audioInputs = async () =>
  (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === "audioinput");

/** マイクの一覧。ふつうのタブでは、許可のあとにしかラベルを読めない。開いたマイクはここで閉じる */
async function readableInputs() {
  const inputs = await audioInputs();
  if (inputs.some((d) => d.label)) return inputs;
  const asking = await openAudio(true);
  try {
    return await audioInputs();
  } finally {
    stopStream(asking);
  }
}

/** ストリームの音を見張る。`heard` は前に聞いてから音が来たか。
 * ページの音声が動いていなければ分からないので、来たとみなす */
function watchSound(stream: MediaStream) {
  const context = new AudioContext();
  const analyser = context.createAnalyser();
  context.createMediaStreamSource(stream).connect(analyser);
  const samples = new Float32Array(analyser.fftSize);
  let sound = false;
  const timer = setInterval(() => {
    analyser.getFloatTimeDomainData(samples);
    sound ||= samples.some((sample) => Math.abs(sample) > SOUND_LEVEL);
  }, SOUND_CHECK_MS);
  return {
    heard() {
      const heard = sound || context.state !== "running";
      sound = false;
      return heard;
    },
    close() {
      clearInterval(timer);
      void context.close();
    },
  };
}

/** Chrome のウィンドウで選べるマイク */
export interface MicChoice {
  deviceId: string;
  label: string;
}

export interface Microphone {
  /** 認識に渡すマイク。なければ Chrome の既定のマイクを使う */
  track?: MediaStreamTrack;
  heardSound?(): boolean;
  /** 選んだマイクを使えない理由 */
  problem?: MicProblem;
  /** 選べるマイク */
  choices: MicChoice[];
  /** 覚えていた選択で開いたマイク */
  chosen?: MicChoice;
  /** 選んだマイクを閉じて、既定のマイクを開く */
  useDefault(): Promise<void>;
  release(): void;
}

/** マイクを開いたままにする。キャプチャ中のページは Chrome に凍結されない。
 * `name` があれば、`chosen`（ウィンドウで選んだマイク）か、その名前のマイクのトラックを `track` で返す。
 * 使えなければ `track` はなく、既定のマイクを開いて理由を `problem` で返す。`log` は調べるための記録 */
export async function openMicrophone(
  name?: string | null,
  log: (line: string) => void = () => {},
  chosen?: MicChoice,
): Promise<Microphone> {
  let opened = () => {};
  let released = false;
  const close = () => {
    opened();
    opened = () => {};
  };
  const useDefault = async () => {
    close();
    const stream = await openAudio(true);
    if (released) stopStream(stream);
    else opened = () => stopStream(stream);
  };
  const release = () => {
    released = true;
    close();
  };
  const fallBack = async (problem?: MicProblem, choices: MicChoice[] = []) => {
    await useDefault();
    return { problem, choices, useDefault, release };
  };

  if (!name) return fallBack();
  const version = Number(chromeBrand()?.version);
  if (version < TRACK_CHROME) {
    log(`Chrome ${version} には start(track) がありません`);
    return fallBack({ type: "oldChrome", version });
  }
  await micPermission();
  const inputs = await readableInputs();
  log(`Chrome のマイク: ${inputs.map((d) => d.label || "（名前なし）").join(" / ")}`);
  const choices = realMics(inputs).map(({ deviceId, label }) => ({ deviceId, label }));
  const remembered =
    choices.find((d) => d.deviceId === chosen?.deviceId) ??
    choices.find((d) => d.label === chosen?.label);
  const mics = findMics(inputs, name);
  const device = remembered ?? mics[0];
  if (!device) {
    log(`『${name}』に当たるマイクがありません`);
    return fallBack({ type: "missing" }, choices);
  }
  if (!remembered && mics.length > 1)
    log(`『${name}』が ${mics.length} 個あるので、先頭を使います`);
  let stream: MediaStream;
  try {
    stream = await openAudio({ deviceId: { exact: device.deviceId } });
  } catch (e) {
    const error = (e as Error).name;
    log(`getUserMedia のエラー: ${error}`);
    return fallBack({ type: "unopenable", error }, choices);
  }
  log(`開きました: ${stream.getAudioTracks()[0]?.label}`);
  const sound = watchSound(stream);
  opened = () => {
    sound.close();
    stopStream(stream);
  };
  return {
    track: stream.getAudioTracks()[0],
    heardSound: sound.heard,
    choices,
    chosen: remembered,
    useDefault,
    release,
  };
}

interface Listener {
  /** 文字が増えなくなってから区切るまでの時間。省くと区切らない（`cut` で区切る） */
  silenceMs?: number;
  /** 途中か確定の文。確定で文が空なら、途中の文を捨てる */
  onText(text: string, final: boolean): void;
  onFatal(error: string): void;
  /** エラーが続いている（聞き直しは続ける）。`undefined` なら直った */
  onTrouble(error?: string): void;
  /** 認識に渡すマイク。省くと Chrome の既定のマイクを使う */
  track?: MediaStreamTrack;
  /** 前に聞いてから `track` に音が来たか */
  heardSound?(): boolean;
  /** `track` が使えないので、既定のマイクに戻した */
  onDefaultMic?(problem: MicProblem): void;
  /** `track` を渡した結果の記録。同じ行は 1 度だけ */
  onMicLog?(line: string): void;
}

/** 止めるまで聞き続ける。`cut` は今の認識を区切り、すぐ聞き直す */
export function listen(lang: string, listener: Listener) {
  const { silenceMs, onText, onFatal, onTrouble, heardSound, onDefaultMic, onMicLog } = listener;
  let { track } = listener;
  const Recognition = recognition()!;
  let stopped = false;
  let current: Recognizer;
  let startedAt = 0;
  let restart: ReturnType<typeof setTimeout>;
  let silence: ReturnType<typeof setTimeout>;
  let stable: ReturnType<typeof setTimeout>;
  /** 続けてエラーで終わった回数 */
  let failures = 0;
  /** 今の認識のエラー */
  let failed: string | undefined;
  /** 確定していない文 */
  let pending = "";
  /** 区切ったあと、聞き直すまでの結果は使わない */
  let cutting = false;
  let stall: ReturnType<typeof setTimeout>;
  /** `track` で結果が届いた */
  let trackWorks = false;
  /** 今の認識で結果が届いた */
  let heardResult = false;
  /** `track` に音が来ているのに、結果が来ないまま終わった回数 */
  let silentTracks = 0;

  const recovered = () => {
    clearTimeout(stable);
    if (failures >= TROUBLE_FAILURES) onTrouble();
    failures = 0;
  };

  const noted = new Set<string>();
  const note = (line: string) => {
    if (noted.has(line)) return;
    noted.add(line);
    onMicLog?.(line);
  };

  const useDefaultMic = (problem: MicProblem) => {
    note(`既定のマイクに戻します: ${JSON.stringify(problem)}`);
    track = undefined;
    onDefaultMic?.(problem);
  };

  /** 今の認識を終えるときに呼ぶ。音が来ているのに聞き取れないトラックは諦める */
  const checkTrack = () => {
    const heard = heardSound?.() ?? true;
    if (!track || trackWorks || heardResult || failed || !heard) return;
    silentTracks++;
    note(`音が来ているのに結果がありません（${silentTracks} 回目）`);
    if (silentTracks >= SILENT_TRACK_LIMIT) useDefaultMic({ type: "silent" });
  };

  /** 止まった認識を捨てて始め直す。遅れて届く知らせは `current` との比較で無視される */
  const replace = (stuck: Recognizer) => {
    if (stuck !== current || stopped) return;
    try {
      stuck.abort();
    } catch {
      // 動いていなければ止めるものがない
    }
    clearTimeout(silence);
    if (pending) onText(pending, true);
    pending = "";
    checkTrack();
    start();
  };

  const start = () => {
    failed = undefined;
    heardResult = false;
    // 抜かれたマイクのトラックは戻らない
    if (track?.readyState === "ended") useDefaultMic({ type: "ended" });
    if (failures) stable = setTimeout(recovered, STABLE_MS);
    // 再開のたびに作り直す。古いインスタンスの遅れた知らせは使わない
    const recognizer = new Recognition();
    current = recognizer;
    recognizer.lang = speechTag(lang);
    recognizer.continuous = true;
    recognizer.interimResults = true;
    // 動いている間だけ見る。終わったあとの聞き直しの待ちは数えない
    const watch = (ms: number) => {
      clearTimeout(stall);
      stall = setTimeout(() => replace(recognizer), ms);
    };
    const alive = () => watch(STALL_MS);
    recognizer.onstart = () => {
      if (recognizer !== current) return;
      alive();
      if (track) note("start(track): 認識が動き始めました");
    };
    recognizer.onaudiostart = recognizer.onstart;
    recognizer.onresult = ({ resultIndex, results }) => {
      if (recognizer !== current) return;
      alive();
      heardResult = true;
      trackWorks ||= !!track;
      if (track) note("start(track): 結果が届きました");
      recovered();
      if (stopped || cutting) return;
      clearTimeout(silence);
      // 確定は新しいものだけ渡す。途中の文は `resultIndex` より前の変わらない分も含めて全体を作る
      let interim = "";
      for (let i = 0; i < results.length; i++) {
        const result = results[i]!;
        if (!result.isFinal) interim += result[0].transcript;
        else if (i >= resultIndex) {
          pending = "";
          onText(result[0].transcript, true);
        }
      }
      if (!interim.trim()) return;
      pending = interim;
      onText(interim, false);
      // 文字が増えなくなったら止めて確定させる。終わったらすぐ聞き直す
      if (silenceMs !== undefined) silence = setTimeout(() => current.stop(), silenceMs);
    };
    recognizer.onerror = ({ error }) => {
      if (recognizer !== current || stopped) return;
      alive();
      if (track) note(`start(track): 認識のエラー ${error}`);
      if (FATAL.has(error)) {
        stopped = true;
        onFatal(error);
      } else if (!EXPECTED.has(error)) {
        failed = error;
        clearTimeout(stable);
      }
    };
    recognizer.onend = () => {
      if (recognizer !== current) return;
      clearTimeout(stall);
      clearTimeout(silence);
      // 認識が区切れたら、確定しなかった文をそのまま確定する。止めたなら捨てる
      if (pending) onText(stopped ? "" : pending, true);
      pending = "";
      if (stopped) return;
      if (failed) {
        if (++failures === TROUBLE_FAILURES) onTrouble(failed);
      } else recovered();
      checkTrack();
      // 隠れているとタイマーが絞られるので、すぐ聞き直せるときは待たない
      const backoff = failures ? Math.min(RETRY_MS * 2 ** failures, MAX_RETRY_MS) : MIN_RESTART_MS;
      const wait = backoff - (Date.now() - startedAt);
      if (wait > 0) restart = setTimeout(start, wait);
      else start();
    };
    startedAt = Date.now();
    cutting = false;
    watch(START_MS);
    try {
      if (track) recognizer.start(track);
      else recognizer.start();
    } catch (e) {
      if (!track) throw e;
      // トラックを受け付けない（終わったトラックなど）
      useDefaultMic({ type: "unopenable", error: (e as Error).name });
      recognizer.start();
    }
  };
  start();
  return {
    cut() {
      if (cutting) return;
      cutting = true;
      pending = "";
      current.stop();
    },
    stop() {
      stopped = true;
      clearTimeout(restart);
      clearTimeout(silence);
      clearTimeout(stable);
      clearTimeout(stall);
      current.stop();
    },
  };
}
