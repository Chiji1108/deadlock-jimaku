// 用語の候補を、Chrome の中の Gemini Nano（Prompt API）に確かめさせる（DESIGN.md 9.7）。
// core が出した問い（候補と選択肢）に、選択肢の番号で答えさせる。文字は PC から出ない
import type { ChromeQuestions } from "../generated/ChromeQuestions";
import type { Decided } from "../generated/Decided";
import type { Availability } from "./speech";

interface Session {
  prompt(input: string, options: { responseConstraint: unknown }): Promise<string>;
  clone(): Promise<Session>;
  destroy(): void;
}

interface Monitor {
  addEventListener(type: "downloadprogress", listener: (e: { loaded: number }) => void): void;
}

interface LanguageModelApi {
  availability(options: unknown): Promise<string>;
  create(options: unknown): Promise<Session>;
}

/** 入力は英語の問いと日本語の文。測ったときと同じ指定にする（experiments/chrome_eval.mjs の LANGUAGES） */
const LANGUAGES = {
  expectedInputs: [{ type: "text", languages: ["en", "ja"] }],
  expectedOutputs: [{ type: "text", languages: ["ja"] }],
};

/** 測った聞き方（experiments/chrome_eval.mjs の DECIDE_SYSTEM）と同じ */
const SYSTEM =
  "You answer one multiple-choice question about a transcript. " +
  'The input is JSON: "state" is the transcript with its context, "question" is what to decide, ' +
  'and "choices" maps each possible answer to its description. ' +
  'Answer with JSON {"choice": ...}: the one key of "choices" that is correct.';

const api = () => (globalThis as { LanguageModel?: LanguageModelApi }).LanguageModel;

/** 使えるか。downloadable はダウンロードすれば使える（クリックの中で `prepareJudge` する）。
 * 使えない PC（GPU やメモリが足りない）と古い Chrome は unavailable */
export async function judgeAvailability(): Promise<Availability> {
  const state = await api()
    ?.availability(LANGUAGES)
    .catch(() => "unavailable");
  if (state === "available") return "available";
  return state === "downloadable" || state === "downloading" ? "downloadable" : "unavailable";
}

/** モデルを用意する（初回は約 4 GB のダウンロード） */
export async function prepareJudge(onProgress: (loaded: number) => void): Promise<Judge> {
  const monitor = (m: Monitor) =>
    m.addEventListener("downloadprogress", ({ loaded }) => onProgress(loaded));
  const base = await api()!.create({
    ...LANGUAGES,
    initialPrompts: [{ role: "system", content: SYSTEM }],
    monitor,
  });
  return new Judge(base);
}

export class Judge {
  /** 文は 1 つずつ確かめる（GPU を取り合わない） */
  private queue: Promise<unknown> = Promise.resolve();

  constructor(private base: Session) {}

  /** 問いの名前 → 選んだ選択肢。答えられなかった問いは省く（core は辞書の一致のままにする） */
  decide(task: ChromeQuestions): Promise<Decided> {
    const decided = this.queue.then(() => this.answer(task));
    this.queue = decided.catch(() => undefined);
    return decided;
  }

  private async answer({ id, state, questions }: ChromeQuestions): Promise<Decided> {
    const choices: Record<string, string> = {};
    for (const question of questions) {
      // 答えは選択肢の番号にする（選択肢の名前や日本語を答えの決まりに入れると、Prompt API がエラーを返す）
      const numbers = question.choices.map((_, i) => String(i + 1));
      const listed = Object.fromEntries(
        question.choices.map((c, i) => [numbers[i], c.description]),
      );
      const schema = {
        type: "object",
        required: ["choice"],
        additionalProperties: false,
        properties: { choice: { type: "string", enum: numbers } },
      };
      const input = JSON.stringify({ state, question: question.instructions, choices: listed });
      // 問いごとに新しい会話にする（前の問いを引きずらない）
      const session = await this.base.clone();
      try {
        const answer = await session.prompt(input, { responseConstraint: schema });
        const picked = numbers.indexOf((JSON.parse(answer) as { choice?: string }).choice ?? "");
        const chosen = question.choices[picked];
        if (chosen) choices[question.name] = chosen.value;
      } catch {
        // 答えられなかった問いは省く
      } finally {
        session.destroy();
      }
    }
    return { id, choices };
  }

  destroy() {
    this.base.destroy();
  }
}
