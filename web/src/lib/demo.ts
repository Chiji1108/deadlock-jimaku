// LP の見本のデータ。見た目はアプリの部品（frontend/src/lib）をそのまま使い、ここでは台詞と辞書だけを用意する
import type { Entity } from "../../../frontend/src/generated/Entity";
import type { RankState } from "../../../frontend/src/generated/RankState";
import type { Segment } from "../../../frontend/src/generated/Segment";
import type { SubtitleDisplay } from "../../../frontend/src/generated/SubtitleDisplay";
import type { Token } from "../../../frontend/src/generated/Token";
import { setGlossary } from "../../../frontend/src/lib/glossary.svelte";

export const LANGS = [
  { value: "en", label: "English" },
  { value: "ru", label: "Русский" },
  { value: "zh-Hans", label: "中文" },
] as const;
type Lang = (typeof LANGS)[number]["value"];

const hero = (icon: string, ja: string, en: string, ru: string, zh: string): Entity => ({
  kind: "hero",
  className: icon,
  names: { ja, en, ru, "zh-Hans": zh },
  icon: `/media/icons/${icon}.webp`,
});
const item = (
  icon: string,
  slot: "vitality" | "spirit",
  ja: string,
  en: string,
  ru: string,
  zh: string,
): Entity => ({ ...hero(icon, ja, en, ru, zh), kind: "item", slot });

const ENTITIES: Record<string, Entity> = {
  doorman: hero("doorman_sm", "ドアマン", "The Doorman", "Швейцар", "门侍"),
  cart: { ...hero("doorman_luggagetrolley", "キャリーカート", "Luggage Cart", "Багажная тележка", "行李车"), kind: "ability" },
  dispel: item("debuff_remover", "vitality", "ディスペルマジック", "Dispel Magic", "Развеивание магии", "驱散魔法"),
  vortex: item("vortex_web", "spirit", "ボルテックスウェブ", "Vortex Web", "Паутинный вихрь", "雷织漩涡"),
  indomitable: item("indomitable", "vitality", "インドミタブル", "Indomitable", "Непреклонность", "不屈之志"),
  bebop: hero("bebop_sm", "ビバップ", "Bebop", "Бибоп", "比波普"),
  krill: hero("digger_sm", "モー＆クリル", "Mo & Krill", "Мо и Мелкий", "莫克双雄"),
};
setGlossary({ clientVersion: 0, languages: [], entities: ENTITIES });

const t = (text: string, ruby?: string): Token => ({ t: "text", text, ruby });
const e = (ref: string, spoken?: string): Token => ({ t: "entity", ref, spoken });

type Line = { source: Token[]; translations: Record<Lang, Token[]>; card: string };

export const LINES: Line[] = [
  {
    source: [e("doorman", "ドアマン"), t("の"), e("cart", "カート"), t("は"), e("dispel", "ディスペル"), t("で"), t("対策", "たいさく"), t("できます。")],
    translations: {
      en: [e("doorman"), t("'s "), e("cart"), t(" can be countered with "), e("dispel"), t(".")],
      ru: [e("doorman"), t(": "), e("cart"), t(" контрится предметом "), e("dispel"), t(".")],
      "zh-Hans": [e("doorman"), t("的"), e("cart"), t("可以用"), e("dispel"), t("应对。")],
    },
    card: "upgrade_reduce_debuff_duration",
  },
  {
    source: [t("ただし、"), e("vortex", "ボルテックスウェブ"), t("には"), e("indomitable", "インドミ"), t("が"), t("必要", "ひつよう"), t("です。")],
    translations: {
      en: [t("However, you'll need "), e("indomitable"), t(" for "), e("vortex"), t(".")],
      ru: [t("Но против предмета "), e("vortex"), t(" нужна "), e("indomitable"), t(".")],
      "zh-Hans": [t("不过，"), e("vortex"), t("需要用"), e("indomitable"), t("来应对。")],
    },
    card: "upgrade_auto_cleanse",
  },
  {
    source: [t("敵", "てき"), t("に"), e("bebop", "ビバップ"), t("や"), e("krill", "モークリ"), t("もいるなら"), t("絶対", "ぜったい"), t("に"), t("買", "か"), t("いましょう。")],
    translations: {
      en: [t("If the enemy team also has "), e("bebop"), t(" or "), e("krill"), t(", definitely buy it.")],
      ru: [t("Если у врагов есть ещё "), e("bebop"), t(" или "), e("krill"), t(", обязательно покупайте.")],
      "zh-Hans": [t("如果敌方还有"), e("bebop"), t("或"), e("krill"), t("，一定要买。")],
    },
    card: "upgrade_auto_cleanse",
  },
];

/** 話した文。途中の字幕はこれを 1 文字ずつ伸ばす */
export const spokenText = (line: Line) =>
  line.source.map((token) => (token.t === "text" ? token.text : (token.spoken ?? ""))).join("");

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
  background: "outline",
  perCharSeconds: 0,
  minReadSeconds: 0,
  maxWaitSeconds: 0,
  maxLines: 1,
  holdSeconds: 0,
  fonts: {},
});

export const RANK: RankState = {
  badge: 73,
  tier: 7,
  subrank: 3,
  names: { ja: "エミッサリー", en: "Emissary" },
  image: "/media/rank-badge.webp",
  progress: { value: 640, max: 1000 },
};
