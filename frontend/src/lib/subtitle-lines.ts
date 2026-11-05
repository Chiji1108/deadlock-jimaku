import type { Segment } from "../generated/Segment";
import type { Settings } from "../generated/Settings";
import type { SubtitleDisplay } from "../generated/SubtitleDisplay";

/** 翻訳だけを出すか。翻訳先がなければ、話した言葉を出す */
export const translationOnly = (s: Settings) =>
  !s.subtitleDisplay.showSource && s.translateTo.length > 0;

/** 届いている訳文。書きかけのうちから出す */
export const translationsOf = (s: Segment) => s.enriched?.translations ?? s.draft ?? {};

/** 翻訳だけを出すときは、翻訳が届くまで字幕を出さない */
export const isReady = (s: Segment, { showSource }: Pick<SubtitleDisplay, "showSource">) =>
  showSource || s.enriched !== undefined || s.draft !== undefined;

/** 話した言葉の行を出すか。翻訳に失敗したら、字幕が消えないよう出す */
export const showsSource = (s: Segment, display?: Pick<SubtitleDisplay, "showSource">) =>
  display?.showSource !== false ||
  !Object.values(translationsOf(s)).some((tokens) => tokens?.length);

type Timing = Pick<SubtitleDisplay, "maxLines" | "minReadSeconds" | "maxWaitSeconds">;

/**
 * 同時に出す数に収める。一番古い行を読み終えていれば消し、まだなら一番新しい行を待たせる。
 * 新しい行が待ちすぎたら、読み終えていなくても古い行を押し出す。`live` は話した順。
 * どちらの条件も時間がたつほど成り立つ方にしか変わらないので、消えた行は戻らない。
 * `maxLines` が 0 なら制限なし
 */
export function visibleLines(
  live: Segment[],
  now: number,
  { maxLines, minReadSeconds, maxWaitSeconds }: Timing,
): Segment[] {
  if (maxLines === 0) return live;
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
