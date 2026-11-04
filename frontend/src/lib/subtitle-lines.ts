import type { Segment } from "../generated/Segment";
import type { SubtitleDisplay } from "../generated/SubtitleDisplay";

type Timing = Pick<SubtitleDisplay, "maxLines" | "minReadSeconds" | "maxWaitSeconds">;

/**
 * 同時に出す数に収める。読み終えた行があればそれを消し、なければ一番新しい行を待たせる。
 * 新しい行が待ちすぎたら、読み終えていなくても古い行を押し出す。`live` は話した順。
 * `gone` は消した行（呼び出し側で捨てる。戻すと一度消えた字幕がまた出てしまう）
 */
export function visibleLines(
  live: Segment[],
  now: number,
  { maxLines, minReadSeconds, maxWaitSeconds }: Timing,
): { shown: Segment[]; gone: Segment[] } {
  const shown = [...live];
  const gone: Segment[] = [];
  while (shown.length > maxLines) {
    const read = shown.findIndex(
      (s) => s.settledAt !== undefined && now - s.settledAt >= minReadSeconds * 1000,
    );
    const waited = now - shown.at(-1)!.startedAt >= maxWaitSeconds * 1000;
    if (read >= 0) gone.push(...shown.splice(read, 1));
    else if (maxWaitSeconds === 0 || waited) gone.push(shown.shift()!);
    else shown.pop();
  }
  return { shown, gone };
}
