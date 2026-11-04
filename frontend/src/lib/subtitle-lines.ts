import type { Segment } from "../generated/Segment";

/** 翻訳が届いてから、この間は次の字幕に押し出さない（読む時間） */
export const MIN_READ_MS = 3000;

/**
 * 同時に出す数に収める。ふつうは古いものから押し出すが、一番古い字幕がまだ読めていない
 * （翻訳が届いていない・届いたばかり）ときは、代わりに新しい途中の字幕を待たせる。
 * `live` は話した順
 */
export function visibleLines(live: Segment[], max: number, now: number): Segment[] {
  const lines = [...live];
  while (lines.length > max) {
    const oldest = lines[0]!;
    const read = oldest.settledAt !== undefined && now - oldest.settledAt >= MIN_READ_MS;
    const waiting = lines.findLastIndex((s) => s.settledAt === undefined);
    if (read || waiting <= 0) lines.shift();
    else lines.splice(waiting, 1);
  }
  return lines;
}
