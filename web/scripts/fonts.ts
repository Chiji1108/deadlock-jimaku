// 文字の見た目の見本（src/lib/looks.ts）と大きい見出し（.display）のフォントを、使う字だけを入れた woff2 にして
// src/assets/fonts に置く。見出しの字はビルドした LP（dist/*.html）の .display から拾うので、先に `astro build` する。
// フォントはカタログ（crates/core/src/font-catalog.json）の google/fonts のコミットから取り、SHA-256 で確かめる。
// --check では作らず、置いてある woff2 がそのフォントと太さで、使う字がすべて入っているかを確かめる
// （見出しは dist があるときだけ。CI とデプロイはビルドしてから確かめる）
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { create, type Font } from "fontkit";
import subsetFont from "subset-font";
import catalog from "../../crates/core/src/font-catalog.json" with { type: "json" };
import { ENTITIES, LINES, RANK } from "../src/lib/lines.ts";
import { fontFile, LOOKS } from "../src/lib/looks.ts";

const OUTPUT = resolve(import.meta.dirname, "../src/assets/fonts");
const DIST = resolve(import.meta.dirname, "../dist");

/** 大きい見出しのフォント。Windows の標準のフォントは 900 の太さがないので、見出しだけ読み込む（global.css の @font-face） */
const DISPLAY = { family: "Noto Sans JP", weight: 900 };
const ORIGIN = `https://raw.githubusercontent.com/google/fonts/${catalog.commit}`;

/** ASCII は文を変えても困らないよう全部入れる */
const ASCII = String.fromCodePoint(...Array.from({ length: 0x7f - 0x20 }, (_, i) => 0x20 + i));
const chars = (text: string) => [...new Set([...ASCII, ...text])].toSorted().join("");

/** 見本の日本語の行とランクに出る字 */
const LOOK_TEXT = chars(
  [
    ...LINES.flatMap((line) => line.source)
      .map((token) => (token.t === "text" ? token.text + (token.ruby ?? "") : (token.spoken ?? "")))
      .join(""),
    Object.values(ENTITIES)
      .map((entity) => entity.names.ja)
      .join(""),
    RANK.names.ja!,
  ].join(""),
);

const decode = (html: string) =>
  html
    .replace(/<[^>]*>/g, "")
    .replace(/&#x([0-9a-f]+);/gi, (_, hex: string) => String.fromCodePoint(parseInt(hex, 16)))
    .replace(/&#(\d+);/g, (_, dec: string) => String.fromCodePoint(Number(dec)))
    .replace(/&(lt|gt|quot|apos|amp);/g, (_, name: string) => ({ lt: "<", gt: ">", quot: '"', apos: "'", amp: "&" })[name]!);

/** class に display を持つ要素の中の字（中の要素も含む） */
function displayText(html: string): string {
  let text = "";
  for (const open of html.matchAll(/<([a-z0-9]+)\b[^>]*\bclass="([^"]*)"[^>]*>/gi)) {
    if (!open[2]!.split(/\s+/).includes("display")) continue;
    const start = open.index + open[0].length;
    const nested = new RegExp(`<(/?)${open[1]}\\b[^>]*>`, "gi");
    let depth = 1;
    for (const m of html.slice(start).matchAll(nested)) {
      depth += m[1] ? -1 : 1;
      if (depth === 0) {
        text += decode(html.slice(start, start + m.index));
        break;
      }
    }
  }
  return text;
}

/** ビルドした LP の見出しの字。dist がなければ undefined */
function displayChars(): string | undefined {
  if (!existsSync(DIST)) return undefined;
  const pages = readdirSync(DIST).filter((name) => name.endsWith(".html"));
  return chars(pages.map((name) => displayText(readFileSync(join(DIST, name), "utf8"))).join(""));
}

function catalogFont(family: string) {
  const entry = catalog.fonts.find((e) => e.family === family);
  if (!entry?.languages.includes("ja")) throw new Error(`${family} はカタログの日本語のフォントにありません`);
  return entry;
}

const fonts: { entry: ReturnType<typeof catalogFont>; weight: number; file: string; text?: string }[] = [
  ...LOOKS.flatMap(({ fontStyle: { font, weight } }) => {
    if (!font) return [];
    if (!weight) throw new Error(`${font} の太さを決めてください`);
    return [{ entry: catalogFont(font), weight, file: fontFile(font, weight), text: LOOK_TEXT }];
  }),
  {
    entry: catalogFont(DISPLAY.family),
    weight: DISPLAY.weight,
    file: fontFile(DISPLAY.family, DISPLAY.weight),
    text: displayChars(),
  },
];

async function download(entry: (typeof fonts)[number]["entry"], file: (typeof entry.files)[number]) {
  const url = `${ORIGIN}/${entry.dir}/${encodeURIComponent(file.name)}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url} を取れません: ${res.status} ${res.statusText}`);
  const data = Buffer.from(await res.arrayBuffer());
  if (createHash("sha256").update(data).digest("hex") !== file.sha256) {
    throw new Error(`${file.name} の SHA-256 が違います`);
  }
  return data;
}

/** 名前の表のライセンス（13）とその URL（14）も残し、配るファイルにライセンスを付ける（OFL） */
const LICENSE = [13, 14];

/** その太さの面。可変フォントは太さを固定する */
async function subset({ entry, weight, file: name, text }: (typeof fonts)[number]) {
  if (!text) throw new Error(`${name} の字を拾えません。先に LP をビルドしてください（pnpm --dir web build）`);
  for (const file of entry.files) {
    const data = await download(entry, file);
    const font = create(data) as Font;
    const axis = font.variationAxes.wght;
    if (axis && axis.min <= weight && weight <= axis.max) {
      return subsetFont(data, text, {
        targetFormat: "woff2",
        preserveNameIds: LICENSE,
        variationAxes: { wght: weight },
      });
    }
    if (!axis && font["OS/2"].usWeightClass === weight) {
      return subsetFont(data, text, { targetFormat: "woff2", preserveNameIds: LICENSE });
    }
  }
  throw new Error(`${entry.family} に太さ ${weight} の面がカタログにありません`);
}

function check({ entry, weight, file, text }: (typeof fonts)[number]) {
  const font = create(readFileSync(join(OUTPUT, file))) as Font;
  // 可変フォントの太さを固定すると、名前は元の既定の面のまま（"Noto Sans JP Thin"）になる。太さは下で確かめる
  if (!font.familyName.startsWith(entry.family)) throw new Error(`${file} は ${entry.family} ではありません`);
  if (font["OS/2"].usWeightClass !== weight) throw new Error(`${file} の太さが ${weight} ではありません`);
  if (!font.getName("license", "en")) throw new Error(`${file} にライセンスがありません`);
  if (!text) {
    console.log(`${file}: LP をビルドしていないので、見出しの字は確かめていません`);
    return;
  }
  const missing = [...text].filter((char) => !font.hasGlyphForCodePoint(char.codePointAt(0)!));
  if (missing.length > 0) {
    throw new Error(`${file} に使う字がありません: ${missing.join("")}（mise run lp-fonts で作り直す）`);
  }
}

const expected = new Set(fonts.map(({ file }) => file));
const stale = readdirSync(OUTPUT).filter((name) => name.endsWith(".woff2") && !expected.has(name));

if (process.argv.includes("--check")) {
  if (stale.length > 0) throw new Error(`使っていないフォントがあります: ${stale.join(", ")}`);
  for (const font of fonts) check(font);
} else {
  // 全部作れてから書き換え、失敗したら今のファイルを残す
  const outputs = await Promise.all(fonts.map(subset));
  for (const name of stale) rmSync(join(OUTPUT, name));
  fonts.forEach(({ file }, i) => {
    writeFileSync(join(OUTPUT, file), outputs[i]!);
    console.log(file, outputs[i]!.length);
  });
  for (const font of fonts) check(font);
}
