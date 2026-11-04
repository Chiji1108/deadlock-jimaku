import { describe, expect, it } from "vitest";
import type { Segment } from "../generated/Segment";
import { visibleLines } from "./subtitle-lines";

const NOW = 100_000;
const TIMING = { minReadSeconds: 3, maxWaitSeconds: 6 };
/** 何ミリ秒前に翻訳が届いたか（なければ翻訳待ち）。話し始めは何ミリ秒前か */
const line = (id: string, settledAgo?: number, startedAgo = 1000) =>
  ({
    id,
    startedAt: NOW - startedAgo,
    settledAt: settledAgo === undefined ? undefined : NOW - settledAgo,
  }) as Segment;
const shown = (lines: Segment[], maxLines: number, timing = TIMING) =>
  visibleLines(lines, NOW, { maxLines, ...timing })
    .shown.map((s) => s.id)
    .join("");

const gone = (lines: Segment[], timing = TIMING) =>
  visibleLines(lines, NOW, { maxLines: 1, ...timing }).gone.map((s) => s.id);

describe("visibleLines", () => {
  it("shows everything while there is room", () => {
    expect(shown([line("a")], 1)).toBe("a");
  });

  it("keeps a line for the reading time, even when the next one is translated", () => {
    expect(shown([line("a", 1000), line("b")], 1)).toBe("a");
    expect(shown([line("a", 1000), line("b", 500)], 1)).toBe("a");
    expect(shown([line("a", 3000), line("b")], 1)).toBe("b");
  });

  it("keeps a line that is not translated yet", () => {
    expect(shown([line("a"), line("b", 100)], 1)).toBe("a");
  });

  it("does not drop a line on screen when translations arrive out of order", () => {
    expect(shown([line("a", 1000), line("b"), line("c")], 2)).toBe("ab");
    expect(shown([line("a", 1000), line("b"), line("c", 100)], 2)).toBe("ab");
  });

  it("removes a read line first, wherever it is", () => {
    expect(shown([line("a"), line("b", 5000), line("c", 5000), line("d")], 3)).toBe("acd");
  });

  it("replaces read lines at once with the default of three", () => {
    expect(shown([line("a", 9000), line("b", 8000), line("c", 7000), line("d")], 3)).toBe("bcd");
  });

  it("replaces at once when waiting is turned off, even before the clock ticks", () => {
    const timing = { minReadSeconds: 3, maxWaitSeconds: 0 };
    expect(shown([line("a", 1000), line("b")], 1, timing)).toBe("b");
    expect(shown([line("a", 1000), line("b", undefined, -200)], 1, timing)).toBe("b");
  });

  it("reports pushed out lines so they do not come back, but not the waiting ones", () => {
    expect(gone([line("a"), line("b")], { minReadSeconds: 3, maxWaitSeconds: 0 })).toEqual(["a"]);
    expect(gone([line("a", 1000), line("b")])).toEqual([]);
  });

  it("uses the configured reading time", () => {
    const timing = { minReadSeconds: 1, maxWaitSeconds: 6 };
    expect(shown([line("a", 1000), line("b")], 1, timing)).toBe("b");
  });

  it("stops holding the newest line when translations take too long", () => {
    expect(shown([line("a"), line("b", undefined, 5999)], 1)).toBe("a");
    expect(shown([line("a"), line("b", undefined, 6000)], 1)).toBe("b");
  });
});
