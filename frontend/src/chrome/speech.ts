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

/** 続けても直らないエラー。それ以外（無音で切れたなど）は認識を始め直す */
const FATAL = new Set([
  "not-allowed",
  "service-not-allowed",
  "language-not-supported",
  "audio-capture",
]);
/** すぐ終わり続けるときに、始め直しを連打しない */
const RETRY_MS = 1000;
const CREATE_ATTEMPTS = 3;
/** 通信できない間の始め直しの間隔の上限。回線が戻ればこの時間のうちに再開する */
const MAX_RETRY_MS = 15_000;
/** 続けて失敗したら、通信できないと知らせる */
const NETWORK_FAILURES = 3;
/** 認識が動いているはずなのに、この時間なにも届かなければ止まったとみなして作り直す */
const STALL_MS = 30_000;
/** マイクのトラックに音が来ているのに結果が来ないまま、この回数作り直したら既定のマイクに戻す。
 * 受け付けても黙って動かない環境がある。黙っているだけなら数えない */
const SILENT_TRACK_LIMIT = 2;
/** トラックの音を見る間隔と、音が来ているとみなす振幅（-40 dBFS。部屋の雑音は数えない） */
const SOUND_CHECK_MS = 250;
const SOUND_LEVEL = 0.01;
/** 通信のエラーが出ずにこの時間続いたら、回線が戻ったとみなす（黙っていると結果が届かない） */
const STABLE_MS = 5000;

const recognition = () => {
  const { SpeechRecognition, webkitSpeechRecognition } = window as unknown as {
    SpeechRecognition?: RecognizerClass;
    webkitSpeechRecognition?: RecognizerClass;
  };
  return SpeechRecognition ?? webkitSpeechRecognition;
};

const translatorApi = () => (window as unknown as { Translator?: TranslatorClass }).Translator;

/** Google Chrome か。Edge や WebView2 は音声の送り先が違い、認識も失敗することがある */
export function supported() {
  const { userAgentData } = navigator as unknown as {
    userAgentData?: { brands: { brand: string }[] };
  };
  return !!recognition() && !!userAgentData?.brands.some((b) => b.brand === "Google Chrome");
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

/** アプリのマイクの名前とブラウザのラベルは同じとは限らないので、片方がもう片方を含めば同じとみなす */
const sameMic = (name: string, label: string) =>
  !!label && (label === name || label.includes(name) || name.includes(label));

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

/** マイクを開いたままにする。キャプチャ中のページは Chrome に凍結されない。
 * `name` があれば、その名前のマイクのトラックを `track` で返す。見つからなければ `track` はなく、
 * 既定のマイクを使う */
export async function openMicrophone(name?: string | null) {
  const open = (audio: MediaStreamConstraints["audio"]) =>
    navigator.mediaDevices.getUserMedia({ audio });
  const closer = (stream: MediaStream) => () => stream.getTracks().forEach((track) => track.stop());
  // ラベルは許可のあとにしか読めない
  const fallback = await open(true);
  const devices = name ? await navigator.mediaDevices.enumerateDevices() : [];
  const inputs = devices.filter((d) => d.kind === "audioinput");
  const device =
    inputs.find((d) => d.label === name) ?? inputs.find((d) => sameMic(name!, d.label));
  if (!device) return { release: closer(fallback), missed: !!name };
  // 選んだマイクを開けない（ほかのアプリが占有しているなど）なら、開けた既定のマイクで続ける
  const stream = await open({ deviceId: { exact: device.deviceId } }).catch(() => undefined);
  if (!stream) return { release: closer(fallback), missed: true };
  closer(fallback)();
  const sound = watchSound(stream);
  const release = () => {
    sound.close();
    closer(stream)();
  };
  return { release, track: stream.getAudioTracks()[0], heardSound: sound.heard, missed: false };
}

interface Listener {
  /** 文字が増えなくなってから区切るまでの時間。省くと区切らない（`cut` で区切る） */
  silenceMs?: number;
  /** 途中か確定の文。確定で文が空なら、途中の文を捨てる */
  onText(text: string, final: boolean): void;
  onFatal(error: string): void;
  /** 通信できなくなった、または戻った */
  onOffline(offline: boolean): void;
  /** 認識に渡すマイク。省くと Chrome の既定のマイクを使う */
  track?: MediaStreamTrack;
  /** 前に聞いてから `track` に音が来たか */
  heardSound?(): boolean;
  /** `track` が使えないので、既定のマイクに戻した */
  onDefaultMic?(): void;
}

/** 止めるまで聞き続ける。`cut` は今の認識を区切り、すぐ聞き直す */
export function listen(lang: string, listener: Listener) {
  const { silenceMs, onText, onFatal, onOffline, heardSound, onDefaultMic } = listener;
  let { track } = listener;
  const Recognition = recognition()!;
  let stopped = false;
  let current: Recognizer;
  let startedAt = 0;
  let restart: ReturnType<typeof setTimeout>;
  let silence: ReturnType<typeof setTimeout>;
  let stable: ReturnType<typeof setTimeout>;
  /** 続けて通信のエラーで終わった回数 */
  let failures = 0;
  /** 今の認識が通信のエラーになった */
  let failed = false;
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
    if (failures >= NETWORK_FAILURES) onOffline(false);
    failures = 0;
  };

  const useDefaultMic = () => {
    track = undefined;
    onDefaultMic?.();
  };

  /** 今の認識を終えるときに呼ぶ。音が来ているのに聞き取れないトラックは諦める */
  const checkTrack = () => {
    const heard = heardSound?.() ?? true;
    if (!track || trackWorks || heardResult || failed || !heard) return;
    if (++silentTracks >= SILENT_TRACK_LIMIT) useDefaultMic();
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
    failed = false;
    heardResult = false;
    if (failures) stable = setTimeout(recovered, STABLE_MS);
    // 再開のたびに作り直す。古いインスタンスの遅れた知らせは使わない
    const recognizer = new Recognition();
    current = recognizer;
    recognizer.lang = speechTag(lang);
    recognizer.continuous = true;
    recognizer.interimResults = true;
    // 動いている間だけ見る。終わったあとの聞き直しの待ちは数えない
    const alive = () => {
      clearTimeout(stall);
      stall = setTimeout(() => replace(recognizer), STALL_MS);
    };
    recognizer.onstart = () => {
      if (recognizer === current) alive();
    };
    recognizer.onaudiostart = recognizer.onstart;
    recognizer.onresult = ({ resultIndex, results }) => {
      if (recognizer !== current) return;
      alive();
      heardResult = true;
      trackWorks ||= !!track;
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
      if (FATAL.has(error)) {
        stopped = true;
        onFatal(error);
      } else if (error === "network") {
        failed = true;
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
        if (++failures === NETWORK_FAILURES) onOffline(true);
      } else recovered();
      checkTrack();
      // 隠れているとタイマーが絞られるので、すぐ聞き直せるときは待たない
      const backoff = Math.min(RETRY_MS * 2 ** failures, MAX_RETRY_MS);
      const wait = backoff - (Date.now() - startedAt);
      if (wait > 0) restart = setTimeout(start, wait);
      else start();
    };
    startedAt = Date.now();
    cutting = false;
    alive();
    try {
      if (track) recognizer.start(track);
      else recognizer.start();
    } catch (e) {
      if (!track) throw e;
      // トラックを受け付けない（終わったトラックなど）
      useDefaultMic();
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
