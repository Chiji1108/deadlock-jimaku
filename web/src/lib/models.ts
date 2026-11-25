// LP の「モデルの測定結果」の表。載せるモデルは `experiments/models.json` に並べ、数字は評価ツールの生の結果
// （`experiments/results/`）からビルドのときに読む（数字を手で写さない）
import chosen from "../../../experiments/models.json";
import furigana from "../../../experiments/results/furigana_eval.json?raw";
// 5 MB あるので、型を推論させずに文字として読む
import terms from "../../../experiments/results/split_eval.terms_cases.json?raw";
import transcribe from "../../../experiments/results/stt_terms.json?raw";

/** 評価ツールが書く要約。工程ごとに、数えるものが違う */
interface Summary {
  /** 測り終えた時刻（UNIX 秒）。この欄ができる前の結果にはない */
  at?: number;
  /** 前の評価ツール（PC の中の文字起こし）は、回数と料金を書かない */
  runs?: number;
  failed: number;
  /** 何も聞かない行（辞書だけ）は null */
  medianMs: number | null;
  p90Ms: number | null;
  usd?: number;
  /** 用語の判定: 合っていた回数。翻訳: 括弧ごと残ったかを見る厳しい数え方 */
  ok?: number;
  /** 翻訳: アプリが訳文から、渡した用語を全部出せた回数（`both` は 2 言語とも） */
  shown?: { en: number; ru: number; both: number };
  /** ふりがな: 正しく読めた回数と、そのうち辞書の読みを直した回数、辞書では合っていた読みを壊した回数 */
  right?: number;
  fixed?: number;
  broken?: number;
  /** 文字起こし: 書けた用語の数と用語の数、違っていた文字の数と正解の文字の数 */
  termsWritten?: number;
  terms?: number;
  characterErrors?: number;
  characters?: number;
}

interface Chosen {
  /** 生の結果のキー */
  key: string;
  name: string;
  /** 送り先 */
  via: string;
  /** 測った日（生の結果に時刻がないものだけ。その結果がリポジトリに入った日） */
  measured?: string;
  /** サーバー（`server/src/tasks.ts`）でのこのモデルの名前。ページが、「おすすめ」のモードが今使っているモデルに印を付けるのに使う */
  server?: string[];
  note?: string;
}

export interface Row extends Omit<Chosen, "key" | "measured">, Omit<Summary, "at"> {
  /** 測った日（日本時間。`2026-11-24`） */
  measured: string;
}

const day = (time: Date) => time.toLocaleDateString("sv-SE", { timeZone: "Asia/Tokyo" });

/** 表の行（`score` の大きい順）と、その表の回数。回数の違う結果は並べられないので、ビルドを止める */
function table(entries: Chosen[], results: string, score: (row: Row) => number) {
  const summaries = JSON.parse(results) as Record<string, { summary: Summary }>;
  const rows = entries
    .map(({ key, measured, ...listed }): Row => {
      const summary = summaries[key]?.summary;
      if (!summary) throw new Error(`${key}: 測定の結果がない`);
      const { at, ...numbers } = summary;
      const measuredDay = at ? day(new Date(at * 1000)) : measured;
      if (!measuredDay) throw new Error(`${key}: 測った日がない`);
      return { ...listed, ...numbers, measured: measuredDay };
    })
    .sort((a, b) => score(b) - score(a) || (a.medianMs ?? 0) - (b.medianMs ?? 0));
  const runs = rows.find((r) => r.runs !== undefined)?.runs;
  if (runs === undefined) throw new Error("回数のある結果がない");
  const other = rows.find((r) => r.runs !== undefined && r.runs !== runs);
  if (other) throw new Error(`${other.name}: ${other.runs} 回の結果は、${runs} 回の結果と並べられない`);
  return { rows, runs };
}

export const TRANSLATE = table(chosen.translate, terms, (r) => r.shown!.both);
export const TERMS = table(chosen.terms, terms, (r) => r.ok!);
export const FURIGANA = table(chosen.furigana, furigana, (r) => r.right!);
export const TRANSCRIBE = table(chosen.transcribe, transcribe, (r) => r.termsWritten!);
