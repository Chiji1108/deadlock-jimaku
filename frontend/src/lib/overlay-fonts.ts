// 言語ごとに選んだフォントを lang.css の --font に渡す
import type { Settings } from "../generated/Settings";

type Fonts = Settings["overlayFonts"];

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

let added = Promise.resolve();

/**
 * 開いたあとに入ったフォントは WebView が名前で描けないので、一覧の見本のために読み込む。
 * faces.css は 1 回ごとに PC の全部のフォントを読むので、重ねず 1 本ずつ読む
 */
export function addFontFaces(family: string) {
  added = added.then(
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

/** フォントの見本。日本語は言語名だと漢字だけになるので、かなも混ぜる */
export const fontSample = (lang: string) => (lang === "ja" ? "あいアイ漢字" : endonym(lang));

/** 選んだフォントのファイルを core から読む。入れたばかりで WebView が知らないフォントも描ける */
const fontImports = (fonts: Fonts) =>
  [...new Set(Object.values(fonts))].map(
    (family) => `@import url(${cssString(facesUrl(family))});`,
  );

/**
 * 値の末尾のカンマで lang.css の標準の候補につなぐ。:lang(es) は es-419 にも一致するので
 * 属性の完全一致で言語を分け、[lang] を重ねて lang.css のリセットより詳細度を上げる。
 * ファイルが読めなければ、同じ名前の PC のフォントと標準の候補に落ちる
 */
export const fontRules = (fonts: Fonts) =>
  [
    ...fontImports(fonts),
    ...Object.entries(fonts).map(
      ([lang, family]) => `[lang][lang=${cssString(lang)}] { --font: ${cssString(family)}, }`,
    ),
  ].join("\n");

let sheet: HTMLStyleElement | undefined;

/** display はフォント以外でも届く。@import を書き直すと読み終えるまで標準に戻るので、同じなら触らない */
export function applyOverlayFonts(fonts: Fonts) {
  sheet ??= document.head.appendChild(document.createElement("style"));
  const css = fontRules(fonts);
  if (sheet.textContent !== css) sheet.textContent = css;
}
