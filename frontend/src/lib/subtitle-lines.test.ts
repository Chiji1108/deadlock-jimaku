import { describe, expect, it } from "vitest";
import type { Segment } from "../generated/Segment";
import { draftTerms, isReady, revealed, showsSource, visibleLines } from "./subtitle-lines";

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
    .map((s) => s.id)
    .join("");

describe("draftTerms", () => {
  it("lists the terms of the live subtitle", () => {
    const tokens = [
      { t: "text", text: "武器" },
      { t: "entity", ref: "item:1" },
    ] as Segment["tokens"];
    expect([...draftTerms({ tokens } as Segment)]).toEqual(["item:1"]);
    expect([...draftTerms({} as Segment)]).toEqual([]);
  });
});

describe("revealed", () => {
  const word = { t: "text", text: "武器", ruby: "ぶき" } as const;
  const term = { t: "entity", ref: "item:1", spoken: "拡マガ" } as const;

  it("keeps whole terms", () => {
    expect(revealed([word, term], 5)).toEqual([word, term]);
  });

  it("shows a term as plain text until all of it is out", () => {
    expect(revealed([word, term], 3)).toEqual([word, { t: "text", text: "拡" }]);
  });

  it("drops what is not out yet", () => {
    expect(revealed([term, word], 3)).toEqual([term]);
  });
});

describe("visibleLines", () => {
  it("shows everything while there is room", () => {
    expect(shown([line("a")], 1)).toBe("a");
  });

  it("shows everything without a limit", () => {
    expect(shown([line("a"), line("b"), line("c"), line("d")], 0)).toBe("abcd");
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

  it("looks at the oldest line only", () => {
    expect(shown([line("a"), line("b", 5000), line("c", 5000), line("d")], 3)).toBe("abc");
  });

  it("replaces read lines at once with the default of three", () => {
    expect(shown([line("a", 9000), line("b", 8000), line("c", 7000), line("d")], 3)).toBe("bcd");
  });

  it("replaces at once when waiting is turned off, even before the clock ticks", () => {
    const timing = { minReadSeconds: 3, maxWaitSeconds: 0 };
    expect(shown([line("a", 1000), line("b")], 1, timing)).toBe("b");
    expect(shown([line("a", 1000), line("b", undefined, -200)], 1, timing)).toBe("b");
  });

  it("does not bring a pushed out line back", () => {
    const timing = { minReadSeconds: 3, maxWaitSeconds: 0 };
    expect(shown([line("a"), line("b", 5000)], 1, timing)).toBe("b");
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

describe("翻訳だけを出すとき", () => {
  const seg = (enriched?: Partial<NonNullable<Segment["enriched"]>>) =>
    ({ enriched: enriched && { source: [], translations: {}, ...enriched } }) as Segment;
  const translated = seg({ translations: { en: [{ t: "text", text: "Hi" }] } });

  it("翻訳が届くまで字幕を出さない", () => {
    expect(isReady(seg(), { showSource: false })).toBe(false);
    expect(isReady(translated, { showSource: false })).toBe(true);
    expect(isReady(seg(), { showSource: true })).toBe(true);
  });

  it("翻訳があれば話した言葉の行を隠す", () => {
    expect(showsSource(translated, { showSource: false })).toBe(false);
    expect(showsSource(translated, { showSource: true })).toBe(true);
  });

  it("書きかけの訳文が届いたら出す", () => {
    const drafting = { draft: { en: [{ t: "text", text: "Hi" }] } } as unknown as Segment;
    expect(isReady(drafting, { showSource: false })).toBe(true);
    expect(showsSource(drafting, { showSource: false })).toBe(false);
  });

  it("翻訳に失敗したら話した言葉を出す", () => {
    expect(showsSource(seg({}), { showSource: false })).toBe(true);
    expect(showsSource(seg({ translations: { en: [] } }), { showSource: false })).toBe(true);
  });
});
