// Chrome の端末内の音声認識と翻訳（Web Speech API と Translator API）。音声は PC の外に出ない

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
  processLocally: boolean;
  onresult: (event: ResultEvent) => void;
  onerror: (event: { error: string }) => void;
  onend: () => void;
  start(): void;
  stop(): void;
}
interface RecognizerClass {
  new (): Recognizer;
  available?(options: LocalOptions): Promise<Availability>;
  install?(options: LocalOptions): Promise<boolean>;
}
interface LocalOptions {
  langs: string[];
  processLocally: true;
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

/** `ja` を認識に渡す形（`ja-JP`）にする */
const speechTag = (lang: string) => {
  const { language, region } = new Intl.Locale(lang).maximize();
  return `${language}-${region}`;
};
const local = (lang: string): LocalOptions => ({ langs: [speechTag(lang)], processLocally: true });
const pairs = (lang: string, targets: string[]) =>
  targets.map((targetLanguage) => ({ sourceLanguage: lang, targetLanguage }));

/** 使えるか。downloadable はダウンロードすれば使える（クリックの中で `prepare` する） */
export async function availability(lang: string, targets: string[]): Promise<Availability> {
  if (!Recognition?.available || !Recognition.install || !TranslatorApi) return "unavailable";
  const all = await Promise.all([
    Recognition.available(local(lang)),
    ...pairs(lang, targets).map((pair) => TranslatorApi.availability(pair)),
  ]);
  if (all.includes("unavailable")) return "unavailable";
  return all.every((a) => a === "available") ? "available" : "downloadable";
}

/** 言語パックと翻訳モデルを入れる。ダウンロードにはクリックが要る */
export async function prepare(lang: string, targets: string[]): Promise<Map<string, Translator>> {
  // 同時に作ると、初回のダウンロード中に失敗することがあるので 1 つずつ作る
  const created = new Map<string, Translator>();
  for (const pair of pairs(lang, targets)) {
    created.set(pair.targetLanguage, await TranslatorApi!.create(pair));
  }
  await Recognition!.install!(local(lang));
  return created;
}

interface Listener {
  onText(text: string, final: boolean): void;
  /** 認識がひと区切りした。確定しなかった文はもう確定しない */
  onEnd(): void;
  onFatal(error: string): void;
}

/** 止めるまで聞き続ける。戻り値は止める関数 */
export function listen(lang: string, { onText, onEnd, onFatal }: Listener): () => void {
  let stopped = false;
  let current: Recognizer;
  let startedAt = 0;

  const start = () => {
    current = new Recognition!();
    current.lang = speechTag(lang);
    current.continuous = true;
    current.interimResults = true;
    current.processLocally = true;
    current.onresult = ({ resultIndex, results }) => {
      let interim = "";
      for (let i = resultIndex; i < results.length; i++) {
        const result = results[i]!;
        if (result.isFinal) onText(result[0].transcript, true);
        else interim += result[0].transcript;
      }
      if (interim) onText(interim, false);
    };
    current.onerror = ({ error }) => {
      if (!FATAL.has(error)) return;
      stopped = true;
      onFatal(error);
    };
    current.onend = () => {
      onEnd();
      if (stopped) return;
      setTimeout(start, Math.max(0, RETRY_MS - (Date.now() - startedAt)));
    };
    startedAt = Date.now();
    current.start();
  };
  start();
  return () => {
    stopped = true;
    current.stop();
  };
}
