import type { Segment } from "../generated/Segment";
import type { SubtitleDisplay } from "../generated/SubtitleDisplay";

type Timing = Pick<SubtitleDisplay, "maxLines" | "minReadSeconds" | "maxWaitSeconds">;

/**
 * 同時に出す数に収める。一番古い行を読み終えていれば消し、まだなら一番新しい行を待たせる。
 * 新しい行が待ちすぎたら、読み終えていなくても古い行を押し出す。`live` は話した順。
 * どちらの条件も時間がたつほど成り立つ方にしか変わらないので、消えた行は戻らない
 */
export function visibleLines(
  live: Segment[],
  now: number,
  { maxLines, minReadSeconds, maxWaitSeconds }: Timing,
): Segment[] {
  const lines = [...live];
  while (lines.length > maxLines) {
    const { settledAt } = lines[0]!;
    const read = settledAt !== undefined && now - settledAt >= minReadSeconds * 1000;
    const waited = Math.max(0, now - lines.at(-1)!.startedAt) >= maxWaitSeconds * 1000;
    if (read || waited) lines.shift();
    else lines.pop();
  }
  return lines;
}
