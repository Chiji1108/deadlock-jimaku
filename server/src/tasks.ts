// 仕事の口（/v2。DESIGN.md 17.4）の中身: 仕事ごとに、どのモデルに、どう頼むか。アプリは「何をしてほしいか」だけを送り、
// モデルと頼み方はここが決める。モデルを替えるのは管理画面から（アプリのリリースは要らない）。
// 頼み方は、同じモデルを自分のキーで使うときのアプリ（crates/core/src/mt.rs）と同じにしてある

import { eq } from "drizzle-orm";
import type { BetaProvider } from "./beta";
import type { Db } from "./db";
import { language } from "./lang";
import { OPENROUTER_FALLBACK } from "./price";
import { tasks } from "./schema";

/** 翻訳の頼みごと。`text` は、用語を `⟦英語名⟧` で囲んだ文。言語は lang.ts が知っているもの */
export interface Translation {
  text: string;
  source: string;
  target: string;
}

/** 仕事を頼めるモデル */
export interface Model {
  label: string;
  provider: BetaProvider;
  /** 外に送る本文（OpenAI 互換の口） */
  body: (asked: Translation) => unknown;
  /** 答えに料金が入っていないときに数える単価（100 万トークンあたり） */
  price: { input: number; output: number };
  /** 答えを待つ時間（ミリ秒）。待たせすぎると、確定の字幕が遅れる */
  timeoutMs: number;
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
} satisfies Record<string, Model>;

/** 仕事と、頼めるモデル、初めのモデル。仕事を足すときは、ここと v2.ts に口を足す */
export const TASKS = {
  translate: { label: "翻訳", models: TRANSLATORS as Record<string, Model>, standard: "hy-mt2" },
} as const;

export type TaskName = keyof typeof TASKS;

export const isTask = (name: string): name is TaskName => Object.hasOwn(TASKS, name);

/** その仕事を頼めるモデルの名前か */
export const isModel = (task: TaskName, model: string) => Object.hasOwn(TASKS[task].models, model);

/** その仕事の今の設定: モデルの名前と中身、動かしているか。管理画面で変えていなければ、初めのモデルで動かす */
export async function taskOf(db: Db, task: TaskName): Promise<{ id: string; model: Model; enabled: boolean }> {
  const row = await db.select().from(tasks).where(eq(tasks.name, task)).get();
  return chosen(task, row);
}

function chosen(task: TaskName, row: { model: string; enabled: boolean } | undefined) {
  const { models, standard } = TASKS[task];
  // 消したモデルの名前が残っていたら、初めのモデルで作る
  const id = row && isModel(task, row.model) ? row.model : standard;
  return { id, model: models[id]!, enabled: row?.enabled ?? true };
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
