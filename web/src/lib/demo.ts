// LP の見本のデータ。見た目はアプリの部品（frontend/src/lib）をそのまま使い、ここでは台詞（lines.ts）に画像を付けて辞書を渡す
import type { Segment } from "../../../frontend/src/generated/Segment";
import type { SubtitleDisplay } from "../../../frontend/src/generated/SubtitleDisplay";
import { setGlossary } from "../../../frontend/src/lib/glossary.svelte";
import rankBadge from "../assets/rank-badge.webp?url";
import type { Token } from "../../../frontend/src/generated/Token";
import { revealed } from "../../../frontend/src/lib/subtitle-lines";
import { ENTITIES, type Lang, type Line, liveTokens, RANK as RANK_TEXT, spokenText } from "./lines";

export { LANGS, LINES, liveTokens, spokenText } from "./lines";

const ICONS = import.meta.glob<string>("../assets/icons/*.webp", {
  eager: true,
  import: "default",
  query: "?url",
});
const entities = Object.fromEntries(
  Object.entries(ENTITIES).map(([ref, entity]) => [
    ref,
    { ...entity, icon: ICONS[`../assets/icons/${entity.className}.webp`] },
  ]),
);
setGlossary({ clientVersion: 0, languages: [], entities });

export const RANK = { ...RANK_TEXT, image: rankBadge };

/** 途中の訳が、話した文字より何文字ぶん遅れるか */
const DRAFT_LAG = 4;
const length = (tokens: Token[]) => spokenText({ source: tokens }).length;

/** 話している途中の字幕（`spoken` 文字まで話した所）。「おすすめ」のモードと同じく、ふりがなと正式な名前の用語、途中の訳が付く */
export function partial(id: string, line: Line, spoken: number, lang: Lang): Segment {
  const live = liveTokens(line);
  // 訳は、話した分だけ、少し遅れて進む。語の途中では切らない（中国語は空白がないので、字で切る）
  const draft = line.draft[lang];
  const total = length(live);
  const progress = spoken === total ? 1 : Math.max(0, spoken - DRAFT_LAG) / total;
  const shown = revealed(draft, Math.floor(length(draft) * progress));
  const last = shown.at(-1);
  if (last?.t === "text" && last !== draft[shown.length - 1] && lang !== "zh-Hans")
    shown[shown.length - 1] = { t: "text", text: last.text.slice(0, last.text.lastIndexOf(" ") + 1) };
  return {
    id,
    startedAt: 0,
    stage: "partial",
    lang: "ja",
    text: spokenText(line).slice(0, spoken),
    tokens: revealed(live, spoken),
    draft: shown.length ? { [lang]: shown } : undefined,
  };
}
export const enriched = (id: string, line: Line, lang?: Lang): Segment => ({
  id,
  startedAt: 0,
  stage: "enriched",
  lang: "ja",
  text: spokenText(line),
  enriched: { source: line.source, translations: lang ? { [lang]: line.translations[lang] } : {} },
});

export const display = (fontSize: number, translationFontSize: number): SubtitleDisplay => ({
  showSource: true,
  furigana: true,
  align: "center",
  fontSize,
  translationFontSize,
  perCharSeconds: 0,
  minReadSeconds: 0,
  maxWaitSeconds: 0,
  maxLines: 1,
  holdSeconds: 0,
});
