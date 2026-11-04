import type { Segment } from "../generated/Segment";

/** 操作パネルの「高度な設定」で変える */
interface Timing {
  /** 翻訳が届いてから、この間は押し出さない（読む時間） */
  minReadSeconds: number;
  /** 新しい字幕を待たせる上限（翻訳が遅くても、今話している字幕を出す） */
  maxWaitSeconds: number;
}

/**
 * 同時に出す数に収める。読み終えた行があればそれを消し、なければ一番新しい行を待たせる。
 * 新しい行が待ちすぎたら、読み終えていなくても古い行を押し出す。`live` は話した順
 */
export function visibleLines(
  live: Segment[],
  max: number,
  now: number,
  { minReadSeconds, maxWaitSeconds }: Timing,
): Segment[] {
  const lines = [...live];
  while (lines.length > max) {
    const read = lines.findIndex(
      (s) => s.settledAt !== undefined && now - s.settledAt >= minReadSeconds * 1000,
    );
    if (read >= 0) lines.splice(read, 1);
    else if (now - lines.at(-1)!.startedAt >= maxWaitSeconds * 1000) lines.shift();
    else lines.pop();
  }
  return lines;
}
