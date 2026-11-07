// 言語ごとに選んだフォントを lang.css の --font に渡す
import type { SubtitleStyle } from "../generated/SubtitleStyle";

/** 制御文字はサーバーが拒否するので、エスケープするのは引用符と \ だけ */
export const cssString = (name: string) => `"${name.replace(/["\\]/g, "\\$&")}"`;

/** その言語での言語名（한국어、Русский）。読めないコードならコードのまま */
export function endonym(lang: string) {
  try {
    return new Intl.DisplayNames([lang], { type: "language" }).of(lang) ?? lang;
  } catch {
    return lang;
  }
}

const facesUrl = (family: string) => `/api/fonts/${encodeURIComponent(family)}/faces.css`;

let faces: Promise<unknown> = Promise.resolve();

/** faces.css は 1 回ごとに PC の全部のフォントを読むので、重ねず 1 本ずつ読む */
function inTurn<T>(read: () => Promise<T>) {
  const reading = faces.then(read);
  faces = reading.catch(() => undefined);
  return reading;
}

/** 開いたあとに入ったフォントは WebView が名前で描けないので、一覧の見本のために読み込む */
export function addFontFaces(family: string) {
  void inTurn(
    () =>
      new Promise<void>((done) => {
        const link = document.createElement("link");
        link.rel = "stylesheet";
        link.href = facesUrl(family);
        link.addEventListener("load", () => done());
        link.addEventListener("error", () => done());
        document.head.append(link);
      }),
  );
}

const WEIGHT_STEPS = [100, 200, 300, 400, 500, 600, 700, 800, 900];

/** faces.css に書いてある太さ。可変フォントは範囲の中の 100 刻み */
export function faceWeights(css: string) {
  const ranges = [...css.matchAll(/font-weight: (\d+) (\d+);/g)].map(
    ([, min, max]) => [Number(min), Number(max)] as const,
  );
  const weights = ranges.flatMap(([min, max]) =>
    min === max ? [min] : WEIGHT_STEPS.filter((weight) => weight >= min && weight <= max),
  );
  return [...new Set(weights)].toSorted((a, b) => a - b);
}

/** そのフォントの持つ太さ。読めなければ reject */
export const fontWeights = (family: string) =>
  inTurn(async () => {
    const res = await fetch(facesUrl(family));
    if (!res.ok) throw new Error(`faces.css: ${res.status}`);
    return faceWeights(await res.text());
  });

/** フォントの見本。日本語は言語名だと漢字だけになるので、かなも混ぜる */
export const fontSample = (lang: string) => (lang === "ja" ? "あいアイ漢字" : endonym(lang));

/** 選んだフォントのファイルを core から読む。入れたばかりで WebView が知らないフォントも描ける */
const fontImports = (families: string[]) =>
  [...new Set(families)].map((family) => `@import url(${cssString(facesUrl(family))});`);

/**
 * 値の末尾のカンマで lang.css の標準の候補につなぐ。:lang(es) は es-419 にも一致するので
 * 属性の完全一致で言語を分け、[lang] を重ねて lang.css のリセットより詳細度を上げる。
 * ファイルが読めなければ、同じ名前の PC のフォントと標準の候補に落ちる
 */
export function subtitleFontRules(languages: SubtitleStyle["languages"]) {
  const fonts = Object.entries(languages).flatMap(([lang, { font }]) =>
    font ? [[lang, font] as const] : [],
  );
  return [
    ...fontImports(fonts.map(([, family]) => family)),
    ...fonts.map(
      ([lang, family]) => `[lang][lang=${cssString(lang)}] { --font: ${cssString(family)}, }`,
    ),
  ].join("\n");
}

/** ランクは 1 つのフォントなので、ランク名の言語によらず全部の言語に当てる */
export const rankFontRules = (family?: string | null) =>
  family
    ? [...fontImports([family]), `[lang][lang] { --font: ${cssString(family)}, }`].join("\n")
    : "";

let sheet: HTMLStyleElement | undefined;

/** display はフォント以外でも届く。@import を書き直すと読み終えるまで標準に戻るので、同じなら触らない */
export function applyFontRules(css: string) {
  sheet ??= document.head.appendChild(document.createElement("style"));
  if (sheet.textContent !== css) sheet.textContent = css;
}
