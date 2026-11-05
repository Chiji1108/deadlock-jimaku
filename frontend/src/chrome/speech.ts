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
interface TranslatorClass {
  availability(pair: Pair): Promise<Availability>;
  create(pair: Pair): Promise<Translator>;
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
async function create(pair: Pair, attempts = CREATE_ATTEMPTS): Promise<Translator> {
  try {
    return await TranslatorApi!.create(pair);
  } catch (e) {
    if (attempts <= 1) throw e;
    await new Promise((resolve) => setTimeout(resolve, RETRY_MS));
    return create(pair, attempts - 1);
  }
}

/** 翻訳モデルを入れる。ダウンロードにはクリックが要る */
export async function prepare(lang: string, targets: string[]): Promise<Map<string, Translator>> {
  const created = new Map<string, Translator>();
  for (const pair of pairs(lang, targets)) {
    created.set(pair.targetLanguage, await create(pair));
  }
  return created;
}

interface Listener {
  /** 文の区切り（`minSilenceMs`）。Chrome の確定を待つより早く字幕を閉じる */
  silenceMs: number;
  onText(text: string, final: boolean): void;
  /** 認識がひと区切りした。確定しなかった文は、ここで確定する */
  onEnd(): void;
  onFatal(error: string): void;
}

/** 止めるまで聞き続ける。戻り値は止める関数 */
export function listen(lang: string, { silenceMs, onText, onEnd, onFatal }: Listener): () => void {
  let stopped = false;
  let current: Recognizer;
  let startedAt = 0;
  let restart: ReturnType<typeof setTimeout>;
  let silence: ReturnType<typeof setTimeout>;

  const start = () => {
    current = new Recognition!();
    current.lang = speechTag(lang);
    current.continuous = true;
    current.interimResults = true;
    current.onresult = ({ resultIndex, results }) => {
      if (stopped) return;
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
      silence = setTimeout(() => current.stop(), silenceMs);
    };
    current.onerror = ({ error }) => {
      if (!FATAL.has(error)) return;
      stopped = true;
      onFatal(error);
    };
    current.onend = () => {
      clearTimeout(silence);
      onEnd();
      if (stopped) return;
      restart = setTimeout(start, Math.max(0, RETRY_MS - (Date.now() - startedAt)));
    };
    startedAt = Date.now();
    current.start();
  };
  start();
  return () => {
    stopped = true;
    clearTimeout(restart);
    clearTimeout(silence);
    current.stop();
  };
}
