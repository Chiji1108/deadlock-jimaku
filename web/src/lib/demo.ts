// LP の見本のデータ。見た目はアプリの部品（frontend/src/lib）をそのまま使い、ここでは台詞（lines.ts）に画像を付けて辞書を渡す
import type { Segment } from "../../../frontend/src/generated/Segment";
import type { SubtitleDisplay } from "../../../frontend/src/generated/SubtitleDisplay";
import { setGlossary } from "../../../frontend/src/lib/glossary.svelte";
import rankBadge from "../assets/rank-badge.webp?url";
import { ENTITIES, type Lang, type Line, RANK as RANK_TEXT, spokenText } from "./lines";

export { LANGS, LINES, spokenText } from "./lines";

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

export const partial = (id: string, text: string): Segment => ({
  id,
  startedAt: 0,
  stage: "partial",
  lang: "ja",
  text,
});
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
