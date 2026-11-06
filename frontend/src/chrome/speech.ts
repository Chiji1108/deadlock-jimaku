// Chrome の音声認識（Web Speech API）。音声は Chrome を通じて Google に送られる

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

/** 続けても直らないエラー。それ以外（無音で切れたなど）は認識を始め直す */
const FATAL = new Set([
  "not-allowed",
  "service-not-allowed",
  "language-not-supported",
  "audio-capture",
]);
/** すぐ終わり続けるときに、始め直しを連打しない */
const RETRY_MS = 1000;
/** 続けて失敗したら、通信できないと判断する */
const NETWORK_FAILURES = 5;

const recognition = () => {
  const { SpeechRecognition, webkitSpeechRecognition } = window as unknown as {
    SpeechRecognition?: RecognizerClass;
    webkitSpeechRecognition?: RecognizerClass;
  };
  return SpeechRecognition ?? webkitSpeechRecognition;
};

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

/** マイクを開いたままにする。キャプチャ中のページは Chrome に凍結されない。戻り値は閉じる関数 */
export async function holdMicrophone() {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  return () => stream.getTracks().forEach((track) => track.stop());
}

interface Listener {
  /** 文字が増えなくなってから区切るまでの時間 */
  silenceMs: number;
  /** 途中か確定の文。確定で文が空なら、途中の文を捨てる */
  onText(text: string, final: boolean): void;
  onFatal(error: string): void;
}

/** 止めるまで聞き続ける。戻り値は止める関数 */
export function listen(lang: string, { silenceMs, onText, onFatal }: Listener) {
  const Recognition = recognition()!;
  let stopped = false;
  let current: Recognizer;
  let startedAt = 0;
  let restart: ReturnType<typeof setTimeout>;
  let silence: ReturnType<typeof setTimeout>;
  let failures = 0;
  /** 確定していない文 */
  let pending = "";

  const start = () => {
    current = new Recognition();
    current.lang = speechTag(lang);
    current.continuous = true;
    current.interimResults = true;
    current.onresult = ({ resultIndex, results }) => {
      failures = 0;
      if (stopped) return;
      clearTimeout(silence);
      let interim = "";
      for (let i = resultIndex; i < results.length; i++) {
        const result = results[i]!;
        if (result.isFinal) {
          pending = "";
          onText(result[0].transcript.trim(), true);
        } else interim += result[0].transcript;
      }
      interim = interim.trim();
      if (!interim) return;
      pending = interim;
      onText(interim, false);
      // 文字が増えなくなったら止めて確定させる。終わったらすぐ聞き直す
      silence = setTimeout(() => current.stop(), silenceMs);
    };
    current.onerror = ({ error }) => {
      if (error === "network") failures++;
      if (!FATAL.has(error) && failures < NETWORK_FAILURES) return;
      stopped = true;
      onFatal(error);
    };
    current.onend = () => {
      clearTimeout(silence);
      // 認識が区切れたら、確定しなかった文をそのまま確定する。止めたなら捨てる
      if (pending) onText(stopped ? "" : pending, true);
      pending = "";
      if (stopped) return;
      // 隠れているとタイマーが絞られるので、すぐ聞き直せるときは待たない
      const wait = RETRY_MS - (Date.now() - startedAt);
      if (wait > 0) restart = setTimeout(start, wait);
      else start();
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
