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
  onresult: (event: ResultEvent) => void;
  onerror: (event: { error: string }) => void;
  onend: () => void;
  start(): void;
  stop(): void;
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

/** マイクを開いたままにする。キャプチャ中のページは Chrome に凍結されない。戻り値は閉じる関数 */
export async function holdMicrophone() {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  return () => stream.getTracks().forEach((track) => track.stop());
}

interface Listener {
  /** 文字が増えなくなってから区切るまでの時間。省くと区切らない（`cut` で区切る） */
  silenceMs?: number;
  /** 途中か確定の文。確定で文が空なら、途中の文を捨てる */
  onText(text: string, final: boolean): void;
  onFatal(error: string): void;
  /** 通信できなくなった、または戻った */
  onOffline(offline: boolean): void;
}

/** 止めるまで聞き続ける。`cut` は今の認識を区切り、すぐ聞き直す */
export function listen(lang: string, { silenceMs, onText, onFatal, onOffline }: Listener) {
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

  const recovered = () => {
    clearTimeout(stable);
    if (failures >= NETWORK_FAILURES) onOffline(false);
    failures = 0;
  };

  const start = () => {
    failed = false;
    if (failures) stable = setTimeout(recovered, STABLE_MS);
    // 再開のたびに作り直す。古いインスタンスの遅れた知らせは使わない
    const recognizer = new Recognition();
    current = recognizer;
    recognizer.lang = speechTag(lang);
    recognizer.continuous = true;
    recognizer.interimResults = true;
    recognizer.onresult = ({ resultIndex, results }) => {
      if (recognizer !== current) return;
      recovered();
      if (stopped || cutting) return;
      clearTimeout(silence);
      let interim = "";
      for (let i = resultIndex; i < results.length; i++) {
        const result = results[i]!;
        if (result.isFinal) {
          pending = "";
          onText(result[0].transcript, true);
        } else interim += result[0].transcript;
      }
      if (!interim.trim()) return;
      pending = interim;
      onText(interim, false);
      // 文字が増えなくなったら止めて確定させる。終わったらすぐ聞き直す
      if (silenceMs !== undefined) silence = setTimeout(() => current.stop(), silenceMs);
    };
    recognizer.onerror = ({ error }) => {
      if (recognizer !== current) return;
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
      clearTimeout(silence);
      // 認識が区切れたら、確定しなかった文をそのまま確定する。止めたなら捨てる
      if (pending) onText(stopped ? "" : pending, true);
      pending = "";
      if (stopped) return;
      if (failed) {
        if (++failures === NETWORK_FAILURES) onOffline(true);
      } else recovered();
      // 隠れているとタイマーが絞られるので、すぐ聞き直せるときは待たない
      const backoff = Math.min(RETRY_MS * 2 ** failures, MAX_RETRY_MS);
      const wait = backoff - (Date.now() - startedAt);
      if (wait > 0) restart = setTimeout(start, wait);
      else start();
    };
    startedAt = Date.now();
    cutting = false;
    recognizer.start();
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
      current.stop();
    },
  };
}
