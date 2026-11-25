// 統計のタブで使う、数の見せ方と、草・時間帯の表の濃さ

/** 0 は何もない日（時間）。1〜4 は、いちばん多いところに対する割合で濃くする。1 つの色（primary）の濃さだけで量を出す */
export const HEAT = [
  "bg-muted",
  "bg-primary/25",
  "bg-primary/50",
  "bg-primary/75",
  "bg-primary",
] as const;

export function heat(value: number, max: number): string {
  if (value <= 0 || max <= 0) return HEAT[0];
  return HEAT[Math.min(4, Math.ceil((value / max) * 4)) as 1 | 2 | 3 | 4];
}

const compact = new Intl.NumberFormat("ja-JP", { notation: "compact", maximumFractionDigits: 1 });

/** 10 万からは「12.7万」「3.4億」に縮める（桁が増えても並びからはみ出さない） */
export function count(n: number): string {
  return n < 100_000 ? n.toLocaleString("ja-JP") : compact.format(n);
}

/** 時間の長さ（0 分から）。100 時間からは分を出さない */
export function hoursAndMinutes(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  if (minutes < 60) return `${minutes} 分`;
  const hours = Math.floor(minutes / 60);
  return hours < 100 ? `${hours} 時間 ${minutes % 60} 分` : `${count(hours)} 時間`;
}
