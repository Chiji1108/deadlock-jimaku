// LP の「モデルの測定結果」の表。載せるモデルは `experiments/models.json` に並べ、数字は評価ツールの生の結果
// （`experiments/results/split_eval.terms_cases.json`）からビルドのときに読む（数字を手で写さない）
import chosen from "../../../experiments/models.json";
// 5 MB あるので、型を推論させずに文字として読む
import results from "../../../experiments/results/split_eval.terms_cases.json?raw";

/** 表に並べる条件。63 文を 3 回ずつ */
export const RUNS = 189;

interface Summary {
  /** 測り終えた時刻（UNIX 秒）。この欄ができる前の結果にはない */
  at?: number;
  runs: number;
  ok: number;
  failed: number;
  medianMs: number;
  p90Ms: number;
  usd: number;
  /** アプリが訳文から、渡した用語を全部出せた回数（翻訳だけ。`both` は 2 言語とも）。`ok` は、括弧ごと残ったかを見る厳しい数え方 */
  shown?: { en: number; ru: number; both: number };
}

interface Chosen {
  /** 生の結果のキー（`MODE/MODEL`） */
  key: string;
  name: string;
  /** 送り先 */
  via: string;
  /** 測った日（生の結果に時刻がないものだけ。その結果がリポジトリに入った日） */
  measured?: string;
  /** サーバー（`server/src/tasks.ts`）でのこのモデルの名前。「おすすめ」のモードが今どれを使っているかを見分ける */
  server?: string[];
  note?: string;
}

export interface Row extends Omit<Chosen, "key" | "measured" | "server">, Omit<Summary, "at" | "runs"> {
  /** ビルドのときに、「おすすめ」のモードが使っていた */
  recommended: boolean;
  /** 測った日（日本時間。`2026-11-24`） */
  measured: string;
}

/** 並べる順を決める数字。翻訳はアプリが用語を出せた回数、用語の判定は合っていた回数 */
const score = (row: Row) => row.shown?.both ?? row.ok;

const summaries = JSON.parse(results) as Record<string, { summary: Summary }>;

/**
 * 「おすすめ」のモードが今使っているモデル。サーバーに聞く（管理画面で替えられるので、手で書くとすぐ古くなる）。
 * ページには、ビルドした日を添える
 */
async function live(): Promise<Record<"translate" | "decide", { id: string }>> {
  const res = await fetch("https://api.jimaku.deadlock.jp/v2/models", { signal: AbortSignal.timeout(30_000) });
  if (!res.ok) throw new Error(`failed to fetch the models in use: ${res.status}`);
  return res.json();
}

const day = (time: Date) => time.toLocaleDateString("sv-SE", { timeZone: "Asia/Tokyo" });

function rows(entries: Chosen[], inUse: string): Row[] {
  if (!entries.some((e) => e.server?.includes(inUse))) throw new Error(`${inUse}: 「おすすめ」が使っているモデルが表にない`);
  return entries
    .map(({ key, measured, server, ...listed }) => {
      const summary = summaries[key]?.summary;
      if (!summary) throw new Error(`${key}: 測定の結果がない`);
      const { at, runs, ...numbers } = summary;
      if (runs !== RUNS) throw new Error(`${key}: ${runs} 回の結果は、${RUNS} 回の結果と並べられない`);
      const measuredDay = at ? day(new Date(at * 1000)) : measured;
      if (!measuredDay) throw new Error(`${key}: 測った日がない`);
      return { ...listed, ...numbers, measured: measuredDay, recommended: server?.includes(inUse) ?? false };
    })
    .sort((a, b) => score(b) - score(a) || a.medianMs - b.medianMs);
}

const inUse = await live();
/** 「おすすめ」の印を確かめた日（ビルドした日） */
export const CHECKED = day(new Date());
export const TRANSLATE = rows(chosen.translate, inUse.translate.id);
export const TERMS = rows(chosen.terms, inUse.decide.id);
