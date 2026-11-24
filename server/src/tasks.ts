// 仕事の口（/v2。DESIGN.md 17.4）の中身: 仕事ごとに、どのモデルに、どう頼むか。アプリは「何をしてほしいか」だけを送り、
// モデルと頼み方はここが決める。モデルを替えるのは管理画面から（アプリのリリースは要らない）。
// 頼み方は、同じモデルを自分のキーで使うときのアプリ（crates/core/src/mt.rs、decide.rs、stt/qwen.rs）と同じにしてある

import { eq } from "drizzle-orm";
import type { BetaProvider } from "./beta";
import type { Db } from "./db";
import { language } from "./lang";
import { OPENROUTER_FALLBACK, QWEN_AUDIO } from "./price";
import { tasks } from "./schema";

/** 翻訳の頼みごと。`text` は、用語を `⟦英語名⟧` で囲んだ文。言語は lang.ts が知っているもの */
export interface Translation {
  text: string;
  source: string;
  target: string;
}

/** 選ばせる頼みごと。`state` は文と前の文、問いごとに、選べる値とその説明 */
export interface Decision {
  state: string;
  questions: { name: string; instructions: string; choices: { value: string; description: string }[] }[];
}

/** 確定の文字起こしの頼みごと。`wav` は base64 の WAV（16kHz・16bit）、`hints` は出てきそうな言葉 */
export interface Transcription {
  wav: string;
  language: string;
  hints: string[];
}

/** 仕事を頼めるモデル */
export interface Model {
  label: string;
  provider: BetaProvider;
  /** 答えに料金が入っていないときに数える単価（100 万トークンあたり） */
  price: { input: number; output: number };
}

/** 翻訳のモデル（OpenAI 互換の口） */
export interface Translator extends Model {
  provider: "qwen" | "openrouter" | "openai";
  body: (asked: Translation) => unknown;
  /** 答えを待つ時間（ミリ秒）。待たせすぎると、確定の字幕が遅れる */
  timeoutMs: number;
}

/** 選ばせるモデル。`path` は、その会社の口 */
export interface Judge extends Model {
  provider: "openrouter" | "perplexity";
  path: string;
  body: (asked: Decision) => unknown;
}

/** 確定の文字起こしのモデル */
export interface Transcriber extends Model {
  path: string;
  body: (asked: Transcription) => unknown;
}

/** 訳し方の指示。これがないと、括弧を外すことが多い（実測。mt.rs の `DOMAINS`） */
const DOMAINS =
  "Casual live commentary by a streamer playing the video game Deadlock, like gamers talk. " +
  "Words in ⟦ ⟧ are game terms (names of heroes, items and abilities); " +
  "keep them exactly as they are, with the brackets.";

/** 翻訳の出力の上限（トークン）。1 発話の訳は、長くても 200 ほど */
export const MAX_TRANSLATION_TOKENS = 512;

const name = (code: string) => language(code)!.name;

const TRANSLATORS = {
  // system はなく、モデルの説明にある「背景を添えて訳す」指示文で送る。設定は説明の勧める値
  "hy-mt2": {
    label: "Hy-MT2（OpenRouter）",
    provider: "openrouter",
    body: ({ text, target }) => ({
      model: "tencent/hy-mt2-30b-a3b",
      messages: [
        {
          role: "user",
          content:
            `[Background Information]\n${DOMAINS}\n\nPlease translate the following text into ${name(target)}, ` +
            `taking the provided background information into consideration.\n\n[Source Text]\n${text}`,
        },
      ],
      temperature: 0.7,
      top_p: 1.0,
      max_tokens: MAX_TRANSLATION_TOKENS,
    }),
    price: OPENROUTER_FALLBACK,
    // 応答は 0.8 秒ほど
    timeoutMs: 2000,
  },
  "qwen-mt-flash": {
    label: "Qwen-MT-Flash",
    provider: "qwen",
    body: ({ text, source, target }) => ({
      model: "qwen-mt-flash",
      messages: [{ role: "user", content: text }],
      translation_options: {
        source_lang: language(source)!.qwen,
        target_lang: language(target)!.qwen,
        domains: DOMAINS,
      },
    }),
    price: { input: 0.16, output: 0.49 },
    // 応答は 0.3 秒ほど
    timeoutMs: 2000,
  },
  // 汎用の LLM。推論はさせない（させると 2 秒を超える）
  "qwen3.8-flash": {
    label: "Qwen3.8 Flash",
    provider: "qwen",
    body: ({ text, source, target }) => ({
      model: "qwen3.8-flash",
      messages: [
        {
          role: "system",
          content: `Translate the ${name(source)} text into ${name(target)}. ${DOMAINS} Reply with the translation only.`,
        },
        { role: "user", content: text },
      ],
      enable_thinking: false,
      max_tokens: MAX_TRANSLATION_TOKENS,
    }),
    price: { input: 0.15, output: 0.47 },
    // 1〜2 秒かかる（遅いときは 3 秒ほど）
    timeoutMs: 4000,
  },
  // OpenRouter が、Gemma を動かしている会社のどれかに回す。送った文を学習や保存に使わない会社だけに限る
  "gemma-4-26b": {
    label: "Gemma 4 26B（OpenRouter）",
    provider: "openrouter",
    body: ({ text, source, target }) => ({
      model: "google/gemma-4-26b-a4b-it",
      messages: [
        {
          role: "system",
          content: `Translate the ${name(source)} text into ${name(target)}. ${DOMAINS} Reply with the translation only.`,
        },
        { role: "user", content: text },
      ],
      reasoning: { enabled: false },
      provider: { data_collection: "deny" },
      max_tokens: MAX_TRANSLATION_TOKENS,
    }),
    // OpenRouter は料金を答えに入れる。入っていないときの単価
    price: { input: 0.09, output: 0.3 },
    // 0.6〜1.2 秒かかる（回される会社で変わる）
    timeoutMs: 3000,
  },
  // OpenAI に直接頼む。指示文は Qwen3.8 Flash と同じ（アプリが自分のキーで頼むときとも同じ。mt.rs）
  "gpt-6-luna": {
    label: "GPT-6 Luna",
    provider: "openai",
    body: ({ text, source, target }) => ({
      model: "gpt-6-luna",
      messages: [
        {
          role: "system",
          content: `Translate the ${name(source)} text into ${name(target)}. ${DOMAINS} Reply with the translation only.`,
        },
        { role: "user", content: text },
      ],
      reasoning_effort: "none",
      max_completion_tokens: MAX_TRANSLATION_TOKENS,
      // 応答を OpenAI に保存させない
      store: false,
    }),
    price: { input: 0.1, output: 0.5 },
    // 1.5〜2 秒かかる（遅いときは 3 秒ほど）
    timeoutMs: 4000,
  },
} satisfies Record<string, Translator>;

/** Jev と同じ形の問い（OpenRouter の口も、Perplexity の口も同じ）。問いは名前ごとの表で、選べる値ごとに説明を書く */
const decision =
  (model: string): Judge["body"] =>
  ({ state, questions }) => ({
    model,
    state,
    questions: Object.fromEntries(
      questions.map(({ name, instructions, choices }) => [
        name,
        { type: "choice", instructions, criteria: Object.fromEntries(choices.map((c) => [c.value, c.description])) },
      ]),
    ),
  });

const JUDGES = {
  "pplx-decider": {
    label: "Perplexity Decider（OpenRouter）",
    provider: "openrouter",
    path: "/systemone",
    body: decision("perplexity/pplx-decider-v1.2"),
    price: OPENROUTER_FALLBACK,
  },
  // 同じモデルを、Perplexity に直接頼む。応答が 0.17 秒ほど速い（DESIGN.md 9.3）。料金は答えに入らないので、トークンから数える
  "pplx-decider-direct": {
    label: "Perplexity Decider（Perplexity）",
    provider: "perplexity",
    path: "/v1/decisions",
    body: decision("pplx-decider-v1.2"),
    price: { input: 0.02, output: 0 },
  },
} satisfies Record<string, Judge>;

/** 文字起こしに添える言葉の上限（文字数）。前の発話の文として渡す。越えた分は Qwen に切られる */
const HINT_CHARS = 400;

const TRANSCRIBERS = {
  "qwen-audio": {
    label: "Qwen-Audio-3.1-ASR-Flash",
    provider: "qwen",
    path: "/api/v1/services/aigc/multimodal-generation/generation",
    body: ({ wav, language, hints }) => {
      let context = "";
      for (const hint of hints) {
        if (context.length + hint.length + 1 > HINT_CHARS) break;
        context += `${hint}、`;
      }
      const audio = { type: "input_audio", input_audio: { data: `data:audio/wav;base64,${wav}` } };
      return {
        model: QWEN_AUDIO.model,
        input: { messages: [{ role: "user", content: context ? [{ type: "input_text", text: context }, audio] : [audio] }] },
        parameters: { format: "wav", sample_rate: "16000", language_hints: [language] },
      };
    },
    price: QWEN_AUDIO,
  },
} satisfies Record<string, Transcriber>;

/** 仕事と、頼めるモデル、初めのモデル。仕事を足すときは、ここと v2.ts に口を足す */
export const TASKS = {
  translate: { label: "翻訳", models: TRANSLATORS as Record<string, Translator>, standard: "hy-mt2" },
  decide: { label: "用語と読みを選ぶ", models: JUDGES as Record<string, Judge>, standard: "pplx-decider" },
  transcribe: { label: "確定の文字起こし", models: TRANSCRIBERS as Record<string, Transcriber>, standard: "qwen-audio" },
} as const;

export type TaskName = keyof typeof TASKS;
type ModelOf<T extends TaskName> = (typeof TASKS)[T]["models"][string];

export const isTask = (name: string): name is TaskName => Object.hasOwn(TASKS, name);

/** その仕事を頼めるモデルの名前か */
export const isModel = (task: TaskName, model: string) => Object.hasOwn(TASKS[task].models, model);

/** その仕事の今の設定: モデルの名前と中身、動かしているか。管理画面で変えていなければ、初めのモデルで動かす */
export async function taskOf<T extends TaskName>(db: Db, task: T) {
  const row = await db.select().from(tasks).where(eq(tasks.name, task)).get();
  return chosen(task, row);
}

function chosen<T extends TaskName>(task: T, row: { model: string; enabled: boolean } | undefined) {
  const { models, standard } = TASKS[task];
  // 消したモデルの名前が残っていたら、初めのモデルで作る
  const id = row && isModel(task, row.model) ? row.model : standard;
  return { id, model: models[id] as ModelOf<T>, enabled: row?.enabled ?? true };
}

/** どの仕事も、今の設定で（管理画面に出す） */
export async function allTasks(db: Db) {
  const rows = await db.select().from(tasks);
  const names = Object.keys(TASKS) as TaskName[];
  return names.map((task) => ({ task, ...chosen(task, rows.find((row) => row.name === task)) }));
}

/** 今頼める仕事（動かしていて、そのモデルの会社のキーがある）。`providers` は、開発者のキーがある会社 */
export async function openTasks(db: Db, providers: BetaProvider[]): Promise<TaskName[]> {
  const all = await allTasks(db);
  return all.filter((t) => t.enabled && providers.includes(t.model.provider)).map((t) => t.task);
}
