// 字幕の作り方。モードで工程ごとの作り方がまとめて決まり、詳しく選ぶと工程ごとに変えられる。
// モードは `crates/core/src/settings.rs` の `Engines` の定数と同じ組み合わせ
// 操作パネルのモードの欄と LP のモードの節が、同じ並びと言い方で出す
import { ENGINE_PROVIDERS, PROVIDERS } from "../generated/catalog";
import type { Engines } from "../generated/Engines";
import type { Provider } from "../generated/Provider";
import type { ServerModel } from "../generated/ServerModel";

type Item = keyof Engines;

export type Option<K extends Item = Item> = {
  value: Engines[K];
  label: string;
  /** 正確さ。工程の `scale` の物差しで測った値（DESIGN.md 3 章、6.4、9.7）。測っていなければ省く */
  accuracy?: string;
  /** 確定の字幕が出るまでに、この工程がかける時間。測っていなければ省く */
  speed?: string;
  /** 話し続けた 1 時間の料金の目安（1 ドル 150 円）。無料なら「無料」。
   * 使う会社は `ENGINE_PROVIDERS`（core が決める）にあるので、ここには書かない */
  price: string;
  /** 選ぶ前に知っておくこと */
  note?: string;
};

/** 工程と、その作り方。無料のものを先に置く。どの選択肢も、正確さ・速さ・料金を同じ物差しで並べる */
export const ITEMS: {
  [K in Item]: { label: string; hint: string; scale: string; options: Option<K>[] };
} = {
  live: {
    label: "途中の字幕",
    hint: "話している間に出す字幕",
    scale: "正確さは、正しく書けた文字の割合です（録音 2 本）",
    options: [
      {
        value: "chrome",
        label: "Chrome",
        accuracy: "93〜96%",
        price: "無料",
        note: "音声は Google に送られる",
      },
      {
        value: "gptLiveTranscribe",
        label: "GPT-Live-Transcribe",
        accuracy: "95〜96%",
        price: "1 時間 約 150 円",
      },
      {
        value: "qwenAudioStreaming",
        label: "Qwen-Audio-3.1-ASR-Flash-Streaming",
        accuracy: "95〜98%",
        price: "1 時間 約 7 円",
        note: "Chrome を使わない。用語のヒントは渡せない",
      },
      {
        value: "haasJa",
        label: "Haas-ja（ローカル）",
        price: "無料",
        note: "日本語だけ。Chrome を使わず、音声は PC から出ない。話している間、PC の中で何度も認識し直すので、PC の負荷は上がる。初回に約 400 MB をダウンロードする。途中の字幕としての正確さは測っていない",
      },
      {
        value: "whisper",
        label: "Whisper（ローカル）",
        price: "無料",
        note: "Chrome を使わず、音声は PC から出ない。ヒーローの名前を教えられ、日本語以外でも出せる。文字が進むのは約 0.5 秒おき（Haas-ja は約 0.3 秒おき）で、話している間の半分ほど GPU を使う。確定も Whisper ならそのモデル、ほかなら large-v3-turbo を使う（初回に約 600 MB をダウンロードする）",
      },
      { value: "off", label: "出さない", price: "無料" },
    ],
  },
  transcription: {
    label: "確定の文字起こし",
    hint: "話し終えたあとに置き換える字幕",
    scale:
      "正確さは、正しく書けた文字の割合です（録音 2 本）。速さは、話し終えてから文字が届くまで",
    options: [
      {
        value: "chrome",
        label: "Chrome",
        accuracy: "93〜96%",
        price: "無料",
        note: "発話の区切りも Chrome が決める。音声は Google に送られる",
      },
      {
        value: "gptTranscribe",
        label: "GPT-Transcribe",
        accuracy: "96〜99%",
        speed: "1.6〜2.3 秒",
        price: "1 時間 約 40 円",
      },
      {
        value: "server",
        label: "デドロ字幕のサーバー",
        price: "ベータ期間中は無料",
        note: "サーバーが、そのときのおすすめのモデルで作る",
      },
      {
        value: "grokVoiceTranscribe",
        label: "Grok Voice Transcribe",
        accuracy: "94〜98%",
        speed: "0.6〜1.0 秒",
        price: "1 時間 約 15 円",
      },
      {
        value: "qwenAudio",
        label: "Qwen-Audio-3.1-ASR-Flash",
        accuracy: "95〜98%",
        speed: "1.3〜3.1 秒",
        price: "1 時間 約 2 円",
      },
      {
        value: "maiTranscribe2",
        label: "MAI-Transcribe 2",
        accuracy: "94〜97%",
        speed: "0.8〜1.5 秒",
        price: "1 時間 約 15 円",
        note: "用語のヒントは渡せない",
      },
      {
        value: "haasJa",
        label: "Haas-ja（ローカル）",
        accuracy: "95〜97%",
        speed: "GPU で約 0.1 秒、CPU だけなら 1〜1.5 秒",
        price: "無料",
        note: "日本語だけ。音声は PC から出ない。用語を教えられないので、カタカナの用語は間違えやすい。初回に約 400 MB をダウンロードする。Intel の Mac では動かない",
      },
      {
        value: "reazonSpeech",
        label: "ReazonSpeech k2（ローカル）",
        accuracy: "86〜96%",
        speed: "CPU で約 0.1 秒",
        price: "無料",
        note: "日本語だけ。音声は PC から出ない。辞書の用語を教えるので、カタカナの用語に強い（合成音声の 207 個のうち 185 個。Haas-ja は 177 個、Chrome は 171 個）。書き方の違い（「バイオレット」を「ヴァイオレット」）もあり、ふだんの言葉の正確さは Chrome より下。句読点は、話の間から付ける（Chrome と同じくらい）。GPU を使わない。初回に約 650 MB をダウンロードする",
      },
      {
        value: "whisperTurbo",
        label: "Whisper large-v3-turbo（ローカル）",
        accuracy: "94〜98%",
        speed: "GPU で約 0.3 秒",
        price: "無料",
        note: "音声は PC から出ない。ヒーローの名前を教えるので、Haas-ja よりカタカナの用語に強い。GPU を使う。初回に約 600 MB をダウンロードする。Intel の Mac では動かない",
      },
      {
        value: "whisperLarge",
        label: "Whisper large-v3（ローカル）",
        accuracy: "95〜98%",
        speed: "GPU で約 0.5 秒",
        price: "無料",
        note: "turbo より用語に強く、約 2 倍遅い。初回に約 1.1 GB をダウンロードする。ほかは turbo と同じ",
      },
    ],
  },
  terms: {
    label: "用語",
    hint: "ヒーロー・スキル・アイテムの判定。翻訳を GPT にしたときは、翻訳と同時に判定するので選びません",
    scale: "正確さは、63 文のテストで、用語を正しく判定できた割合です",
    options: [
      {
        value: "dictionary",
        label: "辞書の一致（ローカル）",
        accuracy: "43%",
        speed: "すぐ",
        price: "無料",
        note: "正式な名前だけ。略称や「〇〇のウルト」は判定しない",
      },
      {
        value: "geminiNano",
        label: "Gemini Nano（Chrome の中）",
        accuracy: "71%",
        speed: "1 問 約 0.4 秒",
        price: "無料",
        note: "文字を PC から出さない。初回に Chrome が約 4 GB をダウンロードし、GPU のメモリ（VRAM）を約 3 GB 使う。使えない PC では辞書の一致になる",
      },
      {
        value: "gemma",
        label: "Gemma 4 E4B（ローカル）",
        accuracy: "83%",
        speed: "GPU で約 0.1 秒",
        price: "無料",
        note: "文字を PC から出さない。GPU のメモリを 5 GB 以上使う。初回に約 5 GB をダウンロードする。Intel の Mac では動かない",
      },
      {
        value: "decisions",
        label: "GPT-6 Luna（Decisions）",
        accuracy: "92%",
        speed: "約 0.25 秒",
        price: "1 時間 約 4 円",
      },
      {
        value: "jev",
        label: "Jev",
        accuracy: "91%",
        speed: "約 0.2 秒",
        price: "1 時間 約 2 円",
      },
      {
        value: "drexMax",
        label: "Drex Max",
        accuracy: "96%",
        speed: "約 0.4 秒",
        price: "1 時間 約 2 円",
      },
      {
        value: "decision1",
        label: "Microsoft Decision 1",
        accuracy: "95%",
        speed: "約 0.5 秒",
        price: "1 時間 約 1 円",
      },
      {
        value: "pplxDecider",
        label: "Perplexity Decider",
        accuracy: "94%",
        speed: "約 0.5 秒",
        price: "1 時間 1 円未満",
      },
      {
        value: "server",
        label: "デドロ字幕のサーバー",
        price: "ベータ期間中は無料",
        note: "サーバーが、そのときのおすすめのモデルで判定する",
      },
    ],
  },
  translation: {
    label: "翻訳",
    hint: "訳文。GPT は用語も同時に判定します（訳文の名前を用語から作るので、1 回で聞きます）",
    scale:
      "正確さは、63 文のテストで、アプリが訳文から用語を出せた割合です（英語 / ロシア語）。料金は 1 言語あたり",
    options: [
      {
        value: "server",
        label: "デドロ字幕のサーバー",
        price: "ベータ期間中は無料",
        note: "サーバーが、そのときのおすすめのモデルで訳す",
      },
      {
        value: "chromeTranslator",
        label: "Chrome の翻訳",
        accuracy: "94% / 30%",
        speed: "すぐ",
        price: "無料",
        note: "ロシア語では、用語の名前まで訳してしまう。ゲームの言い回しは苦手",
      },
      {
        value: "gemma",
        label: "Gemma 4 E4B（ローカル）",
        accuracy: "100% / 100%",
        speed: "GPU で約 0.3 秒",
        price: "無料",
        note: "文字を PC から出さない。GPU のメモリを 5 GB 以上使う（用語と同じものを使う）。初回に約 5 GB をダウンロードする。Intel の Mac では動かない",
      },
      {
        value: "qwenMtFlash",
        label: "Qwen-MT-Flash",
        accuracy: "94% / 88%",
        speed: "約 0.25 秒",
        price: "1 時間 約 3 円",
      },
      {
        value: "hyMt2",
        label: "Hy-MT2",
        accuracy: "99% / 99%",
        speed: "約 0.8 秒",
        price: "1 時間 約 1 円",
      },
      {
        value: "qwen38Flash",
        label: "Qwen3.8 Flash",
        accuracy: "100% / 95%",
        speed: "約 1 秒",
        price: "1 時間 約 2 円",
        note: "汎用の AI に訳させる。回数の上限に当たりにくい。ロシア語では、用語の名前を訳すことがある",
      },
      {
        value: "gpt6LunaMt",
        label: "GPT-6 Luna",
        accuracy: "98% / 98%",
        speed: "約 1.5 秒",
        price: "1 時間 約 2 円",
        note: "汎用の AI に訳させる。用語は Hy-MT2 と同じくらい守るが、遅い",
      },
      {
        value: "gemma26b",
        label: "Gemma 4 26B",
        accuracy: "99% / 99%",
        speed: "約 0.7 秒",
        price: "1 時間 約 1 円",
        note: "汎用の AI に訳させる。用語は Hy-MT2 と同じくらい守り、回数の上限に当たりにくい。OpenRouter が、送った文を残さない会社だけに回す",
      },
      {
        value: "gpt61Terra",
        label: "GPT-6.1 Terra（用語も）",
        accuracy: "99% / 99%",
        speed: "約 2.5 秒",
        price: "1 時間 約 45 円",
        note: "訳がいちばん自然。用語も同時に判定する（用語の正確さは 98%）",
      },
    ],
  },
  furigana: {
    label: "ふりがな",
    hint: "日本語の漢字の読み",
    scale:
      "正確さは、読み分けの難しい 60 文のテストで、正しく読めた割合です（ふだんの文は、辞書だけでもほぼ読めます）",
    options: [
      {
        value: "dictionary",
        label: "辞書（ローカル）",
        accuracy: "85%",
        speed: "すぐ",
        price: "無料",
        note: "文の意味で読みが変わる語（辛い、人気）は間違えることがある",
      },
      {
        value: "server",
        label: "辞書 + デドロ字幕のサーバー",
        price: "ベータ期間中は無料",
        note: "読みが分かれる語を、辞書の候補からサーバーが選ぶ",
      },
      {
        value: "decider",
        label: "辞書 + Perplexity Decider",
        accuracy: "95%",
        speed: "約 0.5 秒",
        price: "1 時間 1 円未満",
        note: "読みが分かれる語を、辞書の候補から選ぶ。まれに、合っていた読みを替えてしまう",
      },
      {
        value: "llm",
        label: "辞書 + GPT-6.1 Terra",
        accuracy: "100%",
        speed: "約 1.5 秒",
        price: "1 時間 約 35 円",
        note: "辞書の読みを、文に合わせて直す",
      },
      {
        value: "luna",
        label: "辞書 + GPT-6 Luna",
        accuracy: "99%",
        speed: "約 1.2 秒",
        price: "1 時間 約 4 円",
        note: "辞書の読みを、文に合わせて直す",
      },
    ],
  },
};

/** モードの組み合わせ */
export const CHROME: Engines = {
  live: "chrome",
  transcription: "chrome",
  terms: "dictionary",
  translation: "chromeTranslator",
  furigana: "dictionary",
};
/** ベータ期間中は無料の組み合わせ。途中の字幕は Chrome、ほかは開発者のサーバーに頼む。モデルは、サーバーが決める（DESIGN.md 17.4） */
export const BETA: Engines = {
  live: "chrome",
  transcription: "server",
  terms: "server",
  translation: "server",
  furigana: "server",
};

/** 最高精度の組み合わせ（0.5 までの「スタンダード」）。確定の文字起こし・翻訳と用語・ふりがなを GPT で作る */
export const BEST: Engines = {
  live: "chrome",
  transcription: "gptTranscribe",
  terms: "dictionary",
  translation: "gpt61Terra",
  furigana: "llm",
};

/** Chrome+ のモード。Chrome のモードに、Chrome の中の Gemini Nano での用語の判定を足す */
export const CHROME_PLUS: Engines = { ...CHROME, terms: "geminiNano" };

/** クラウドのモード。「おすすめ」の途中の字幕を Qwen で作り、Chrome を使わない。途中の字幕には、Qwen の自分のキーが要る */
export const CLOUD: Engines = { ...BETA, live: "qwenAudioStreaming" };

/** 音声も字幕の文字も PC から出さない組み合わせ。3 つとも PC の中で動かす（DESIGN.md 9.7） */
export const LOCAL: Engines = {
  live: "haasJa",
  transcription: "whisperTurbo",
  terms: "gemma",
  translation: "gemma",
  furigana: "dictionary",
};

/** 月の目安の使い方（毎日 3 時間、30 日） */
const MONTHLY_HOURS = 3 * 30;
/** 月の目安を円にするときの 1 ドル（為替で動くので目安） */
export const YEN_PER_USD = 150;

/** 月の目安。「毎日 3 時間で月 約 ¥8,100（$54）」 */
export function monthly(hourlyUsd: number) {
  const usd = hourlyUsd * MONTHLY_HOURS;
  const yen = Math.round((usd * YEN_PER_USD) / 100) * 100;
  return `毎日 3 時間で月 約 ¥${yen.toLocaleString("ja-JP")}（$${Math.round(usd)}）`;
}

/** おすすめの組み合わせ。`crates/core/src/settings.rs` の `Engines::MODES` と同じ。ほかの組み合わせは「カスタム」 */
export const MODES: {
  name: string;
  label: string;
  /** 選ぶ理由と条件。名前の横に、短い札で出す */
  traits: string[];
  /** 1 時間の目安（DESIGN.md 3.3）。無料なら 0 */
  hourlyUsd: number;
  hint: string;
  engines: Engines;
  /** 料金の代わりに出すひとこと */
  badge?: string;
  /** 「その他のモード」を開くと出る（選ぶ人が限られる。LP には出さない） */
  more?: true;
}[] = [
  {
    name: "beta",
    label: "おすすめ",
    traits: ["高精度"],
    hourlyUsd: 0,
    hint: "Chrome と、デドロ字幕のサーバーを使って作ります。1 日の上限に届いても、Chrome の作り方で続きます",
    engines: BETA,
    badge: "ベータ期間中は無料",
  },
  {
    name: "chrome",
    label: "Chrome",
    traits: ["サーバーを使わない"],
    hourlyUsd: 0,
    hint: "多くの字幕ソフトと同じく、Chrome の音声認識と翻訳だけで作ります",
    engines: CHROME,
  },
  {
    name: "chromePlus",
    label: "Chrome+",
    traits: ["サーバーを使わない", "VRAM 8 GB 以上推奨"],
    hourlyUsd: 0,
    hint: "Chrome のモードに、Chrome 内蔵の AI の用語の判定を足します。略称や「〇〇のウルト」も拾います。初回に約 4 GB をダウンロードします",
    engines: CHROME_PLUS,
  },
  {
    name: "best",
    label: "最高精度",
    traits: ["OpenAI のキーが要る"],
    hourlyUsd: 0.6,
    hint: "途中の字幕は Chrome、確定の字幕・翻訳・ふりがなは OpenAI の GPT で作ります。いちばん正確ですが、確定の字幕が出るまで 1〜2 秒長くかかります",
    engines: BEST,
    more: true,
  },
  {
    name: "cloud",
    label: "クラウド",
    traits: ["使用メモリ最小", "Qwen のキーが要る"],
    hourlyUsd: 0.05,
    hint: "Chrome を開かず、途中の字幕を Qwen で作ります（Qwen の API キーが要ります）。ほかは「おすすめ」と同じで、デドロ字幕のサーバーに頼みます。音声を Google に送りません",
    engines: CLOUD,
    more: true,
  },
  {
    name: "local",
    label: "ローカル",
    traits: ["外に送らない", "VRAM 12 GB 以上推奨"],
    hourlyUsd: 0,
    hint: "Chrome を開かず、この PC の中の AI だけで作ります。VRAM を 6 GB 以上使います（ゲームと合わせて 12 GB 以上を推奨）。初回に約 6 GB をダウンロードします",
    engines: LOCAL,
    more: true,
  },
];

/** 翻訳が GPT なら、用語も GPT が同時に判定する（用語の選択は使わない） */
export const llmTranslation = (t: Engines) => t.translation === "gpt61Terra";

/** 実際に使う用語の作り方 */
const effective = (t: Engines): Engines => (llmTranslation(t) ? { ...t, terms: "dictionary" } : t);

const same = (a: Engines, b: Engines) =>
  (Object.keys(ITEMS) as Item[]).every((item) => effective(a)[item] === effective(b)[item]);

/** 今の選び方に合うモード。詳しく選んで変えたなら undefined */
export const modeOf = (t: Engines) => MODES.find((m) => same(m.engines, t));

/** 途中の字幕をアプリが作る（有料の音声認識につなぐ） */
export const paidLive = (t: Engines) => providerOfOption("live", t.live) !== null;

/** Chrome のウィンドウを使う（途中の字幕が Chrome） */
export const usesChrome = (t: Engines) => t.live === "chrome";

/** 発話の区切りを Chrome が決める（アプリはマイクを開かない） */
export const chromeFinals = (t: Engines) => t.transcription === "chrome";

/** その選択肢が使う会社（その鍵が要る）。無料なら null */
export const providerOfOption = <K extends Item>(item: K, value: Engines[K]): Provider | null =>
  (ENGINE_PROVIDERS[item] as Record<string, Provider | null>)[value] ?? null;

const providerOf = (t: Engines, item: Item) => providerOfOption(item, effective(t)[item]);

/** 有料の工程で使う会社（その鍵が要る）。OpenAI を先に置く */
export const providers = (t: Engines): Provider[] =>
  PROVIDERS.map((p) => p.id).filter((p) =>
    (Object.keys(ITEMS) as Item[]).some((item) => providerOf(t, item) === p),
  );

/** 有料の工程がある（鍵が要る） */
export const anyPaid = (t: Engines) => providers(t).length > 0;

/** 会社の名前 */
export const PROVIDER_NAMES = Object.fromEntries(PROVIDERS.map((p) => [p.id, p.name])) as Record<
  Provider,
  string
>;

/** デドロ字幕のサーバーに、仕事として頼む工程がある（DESIGN.md 17.4）。会社のキーは使わず、ベータのコードが要る */
export const usesServer = (t: Engines) =>
  (Object.keys(ITEMS) as Item[]).some((item) => effective(t)[item] === "server");

/** サーバーが仕事ごとに今使っているモデル（`GET /v2/models`） */
export type ServerModels = Record<string, ServerModel>;

/** サーバーに頼める工程: サーバーの仕事の名前と、サーバーのモデルを自分のキーで使う選択肢。
 * モデルの名前は `server/src/tasks.ts` のもの */
const SERVER_TASKS: { [K in Item]?: { task: string; same: Record<string, Engines[K]> } } = {
  transcription: { task: "transcribe", same: { "qwen-audio": "qwenAudio" } },
  terms: {
    task: "decide",
    same: { "pplx-decider": "pplxDecider", "pplx-decider-direct": "pplxDecider" },
  },
  translation: {
    task: "translate",
    same: {
      "hy-mt2": "hyMt2",
      "qwen-mt-flash": "qwenMtFlash",
      "qwen3.8-flash": "qwen38Flash",
      "gpt-6-luna": "gpt6LunaMt",
      "gemma-4-26b": "gemma26b",
    },
  },
  furigana: {
    task: "read",
    same: { "pplx-decider": "decider", "pplx-decider-direct": "decider" },
  },
};

/** 画面に出す選択肢。「デドロ字幕のサーバー」には、サーバーが今使っているモデルの名前と、同じモデルの選択肢の正確さと速さを足す。
 * モデルはサーバーが替えるので、数字はここに書かない。モデルが分からない間と、知らないモデルのときは、数字を出さない */
export function shown<K extends Item>(
  item: K,
  option: Option<K>,
  models: ServerModels | undefined,
): Option<K> & { model?: string } {
  const spec = SERVER_TASKS[item];
  const model = option.value === "server" && spec ? models?.[spec.task] : undefined;
  if (!spec || !model) return option;
  const same = ITEMS[item].options.find((o) => o.value === spec.same[model.id]);
  return { ...option, accuracy: same?.accuracy, speed: same?.speed, model: model.label };
}

/** 足りない鍵を入れに行くボタンの文 */
export const ENTER_KEYS = "API キーを入れる";

/** 要る鍵の名前 */
export const keyNames = (list: Provider[]) =>
  `${list.map((p) => PROVIDER_NAMES[p]).join(" と ")} の API キー`;

/** 今の仕組みで作れない選び方。core の `Engines::sanitized` と同じ決まり。
 * Chrome の翻訳を使っている間は途中の字幕を Chrome から変えさせない（変えると core が翻訳を有料に直し、黙って料金がかかる） */
export function unavailable<K extends Item>(t: Engines, item: K, value: Engines[K]) {
  if (item === "live")
    return (chromeFinals(t) || t.translation === "chromeTranslator") && value !== "chrome";
  if (item === "translation") return !usesChrome(t) && value === "chromeTranslator";
  // Gemini Nano は Chrome のページの中にある
  if (item === "terms") return llmTranslation(t) || (value === "geminiNano" && !usesChrome(t));
  return false;
}

/**
 * 正確さを、一目で比べられる 3 段階にする（値の初めの数字で決める。90% 以上は ◎、70% 以上は ○、それより下は △）。
 * 数字のないもの（GPT が翻訳と同時に判定する用語）は、いちばん正確なので ◎
 */
function grade(accuracy: string) {
  const percent = Number(/\d+/.exec(accuracy)?.[0] ?? 100);
  if (percent >= 90) return "◎";
  return percent >= 70 ? "○" : "△";
}

/**
 * モードを見比べるための、工程ごとの正確さ（その工程で選んだ選択肢の、測った値。物差しは工程ごとに違う）。
 * 翻訳を GPT にしたモードは、用語を GPT が翻訳と同時に判定するので、用語の値はない。
 * サーバーに頼む工程は、サーバーが今使っているモデル（`models`）の値。分からなければ、その工程は出さない
 */
export function modeStats(
  t: Engines,
  models?: ServerModels,
): { label: string; value: string; grade: string }[] {
  const accuracy = <K extends Item>(item: K) => {
    const option = ITEMS[item].options.find((o) => o.value === t[item]);
    return option && shown(item, option, models).accuracy;
  };
  const terms = llmTranslation(t) ? "GPT が判定" : accuracy("terms");
  return [
    { label: "文字起こし", value: accuracy("transcription") },
    { label: "用語", value: terms },
    { label: "ふりがな", value: accuracy("furigana") },
  ].flatMap((stat) =>
    stat.value ? [{ label: stat.label, value: stat.value, grade: grade(stat.value) }] : [],
  );
}

/** 有料の工程の名前（詳しく選んだときの説明に使う） */
export const paidItems = (t: Engines) =>
  (Object.keys(ITEMS) as Item[])
    .filter((item) => providerOf(t, item))
    .map((item) => ITEMS[item].label);

/** モードの値段。「無料」か「約 $0.6／時間」 */
export const price = (hourlyUsd: number) => (hourlyUsd ? `約 $${hourlyUsd}／時間` : "無料");
