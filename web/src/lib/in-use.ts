// 「おすすめ」のモードが今使っているモデル。管理画面で替えられるので、ビルドのときではなく、ページを開くたびに
// サーバーに聞く。ページの表は、同じ 1 回の答えを分け合う

/** 仕事の名前（`translate`、`decide`、`transcribe`）ごとの、モデルの名前（`server/src/tasks.ts`）と表示名 */
export type InUse = Record<string, { id: string; label: string } | undefined>;

let asked: Promise<InUse> | undefined;

/** 読めなければ空（印なしのまま、表はそのまま読める） */
export const inUse = () =>
  (asked ??= fetch("https://api.jimaku.deadlock.jp/v2/models", { signal: AbortSignal.timeout(5000) })
    .then((res) => (res.ok ? (res.json() as Promise<InUse | null>) : null))
    .then((answer) => answer ?? {})
    .catch(() => ({})));
