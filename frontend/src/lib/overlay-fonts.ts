// 言語ごとに選んだフォントを lang.css の --font に渡す

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

/** 開いたあとに入ったフォントは WebView が名前で描けないので、一覧の見本のために読み込む */
export function addFontFaces(family: string) {
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = facesUrl(family);
  document.head.append(link);
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
export async function fontWeights(family: string) {
  const res = await fetch(facesUrl(family));
  if (!res.ok) throw new Error(`faces.css: ${res.status}`);
  return faceWeights(await res.text());
}

/** フォントの見本。日本語は言語名だと漢字だけになるので、かなも混ぜる */
export const fontSample = (lang: string) => (lang === "ja" ? "あいアイ漢字" : endonym(lang));

/**
 * lang 属性を持つ要素に書く --font の値。末尾のカンマで lang.css の標準の候補につなぎ、
 * PC にないフォントは標準に落ちる。選んでいなければ書かず、lang.css が initial に戻す
 */
export const fontVar = (family?: string | null) => (family ? `${cssString(family)},` : undefined);

/** 選んだフォントのファイルを core から読む。入れたばかりで WebView が知らないフォントも描ける */
export const fontImports = (families: (string | null)[]) =>
  [...new Set(families)]
    .filter((family) => family !== null)
    .map((family) => `@import url(${cssString(facesUrl(family))});`)
    .join("\n");

let sheet: HTMLStyleElement | undefined;

/** display はフォント以外でも届く。@import を書き直すと読み終えるまで標準に戻るので、同じなら触らない */
export function applyFontImports(css: string) {
  sheet ??= document.head.appendChild(document.createElement("style"));
  if (sheet.textContent !== css) sheet.textContent = css;
}
