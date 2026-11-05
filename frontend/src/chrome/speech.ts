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

const { SpeechRecognition: Recognition, Translator: TranslatorApi } = window as unknown as {
  SpeechRecognition?: RecognizerClass;
  Translator?: TranslatorClass;
};

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
/** 続けて失敗したら、通信できないと判断する */
const NETWORK_FAILURES = 5;

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
  if (!Recognition || !TranslatorApi) {
    return { state: "unavailable" as Availability, targets: [] as string[] };
  }
  const translation = await Promise.all(
    pairs(lang, targets).map((pair) => TranslatorApi.availability(pair)),
  );
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
    return await TranslatorApi!.create({ ...pair, monitor });
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

interface Listener {
  /** 文字が増えなくなってから区切るまでの時間。省くと区切らない（`cut` で区切る） */
  silenceMs?: number;
  onText(text: string, final: boolean): void;
  /** 認識がひと区切りした。確定しなかった文は、ここで確定する */
  onEnd(): void;
  onFatal(error: string): void;
}

interface Listening {
  /** 今の認識を区切り、すぐ聞き直す */
  cut(): void;
  stop(): void;
}

/** マイクを開いたままにする。キャプチャ中のページは Chrome に凍結されない。戻り値は閉じる関数 */
export async function holdMicrophone() {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  return () => stream.getTracks().forEach((track) => track.stop());
}

/** 止めるまで聞き続ける */
export function listen(lang: string, { silenceMs, onText, onEnd, onFatal }: Listener): Listening {
  let stopped = false;
  let current: Recognizer;
  let startedAt = 0;
  let restart: ReturnType<typeof setTimeout>;
  let silence: ReturnType<typeof setTimeout>;
  /** 区切ったあと、聞き直すまでの結果は使わない */
  let cutting = false;
  let failures = 0;

  const start = () => {
    current = new Recognition!();
    current.lang = speechTag(lang);
    current.continuous = true;
    current.interimResults = true;
    current.onresult = ({ resultIndex, results }) => {
      failures = 0;
      if (stopped || cutting) return;
      clearTimeout(silence);
      let interim = "";
      for (let i = resultIndex; i < results.length; i++) {
        const result = results[i]!;
        if (result.isFinal) onText(result[0].transcript, true);
        else interim += result[0].transcript;
      }
      if (!interim) return;
      onText(interim, false);
      // 文字が増えなくなったら止めて確定させる。終わったらすぐ聞き直す
      if (silenceMs !== undefined) silence = setTimeout(() => current.stop(), silenceMs);
    };
    current.onerror = ({ error }) => {
      if (error === "network") failures++;
      if (!FATAL.has(error) && failures < NETWORK_FAILURES) return;
      stopped = true;
      onFatal(error);
    };
    current.onend = () => {
      clearTimeout(silence);
      onEnd();
      if (stopped) return;
      // 隠れているとタイマーが絞られるので、すぐ聞き直せるときは待たない
      const wait = RETRY_MS - (Date.now() - startedAt);
      if (wait > 0) restart = setTimeout(start, wait);
      else start();
    };
    startedAt = Date.now();
    cutting = false;
    current.start();
  };
  start();
  return {
    cut() {
      if (cutting) return;
      cutting = true;
      current.stop();
    },
    stop() {
      stopped = true;
      clearTimeout(restart);
      clearTimeout(silence);
      current.stop();
    },
  };
}
