// 文字の見た目の見本（src/lib/looks.ts）のフォントを、見本の字だけを入れた woff2 にして src/assets/fonts に置く。
// フォントはカタログ（crates/core/src/font-catalog.json）の google/fonts のコミットから取り、SHA-256 で確かめる。
// --check では作らず、置いてある woff2 が見本のフォントと太さで、見本の字がすべて入っているかを確かめる
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { create, type Font } from "fontkit";
import subsetFont from "subset-font";
import catalog from "../../crates/core/src/font-catalog.json" with { type: "json" };
import { ENTITIES, LINES, RANK } from "../src/lib/lines.ts";
import { fontFile, LOOKS } from "../src/lib/looks.ts";

const OUTPUT = resolve(import.meta.dirname, "../src/assets/fonts");
const ORIGIN = `https://raw.githubusercontent.com/google/fonts/${catalog.commit}`;

/** 見本の日本語の行とランクに出る字。ASCII は台詞を変えても困らないよう全部入れる */
const ASCII = String.fromCodePoint(...Array.from({ length: 0x7f - 0x20 }, (_, i) => 0x20 + i));
const TEXT = [
  ...new Set([
    ...ASCII,
    ...LINES.flatMap((line) => line.source)
      .map((token) => (token.t === "text" ? token.text + (token.ruby ?? "") : (token.spoken ?? "")))
      .join(""),
    ...Object.values(ENTITIES)
      .map((entity) => entity.names.ja)
      .join(""),
    ...RANK.names.ja!,
  ]),
]
  .toSorted()
  .join("");

const fonts = LOOKS.flatMap(({ fontStyle: { font, weight } }) => {
  if (!font) return [];
  const entry = catalog.fonts.find((e) => e.family === font);
  if (!entry?.languages.includes("ja")) throw new Error(`${font} はカタログの日本語のフォントにありません`);
  if (!weight) throw new Error(`${font} の太さを決めてください`);
  return [{ entry, weight, file: fontFile(font, weight) }];
});

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
async function subset({ entry, weight }: (typeof fonts)[number]) {
  for (const file of entry.files) {
    const data = await download(entry, file);
    const font = create(data) as Font;
    const axis = font.variationAxes.wght;
    if (axis && axis.min <= weight && weight <= axis.max) {
      return subsetFont(data, TEXT, {
        targetFormat: "woff2",
        preserveNameIds: LICENSE,
        variationAxes: { wght: weight },
      });
    }
    if (!axis && font["OS/2"].usWeightClass === weight) {
      return subsetFont(data, TEXT, { targetFormat: "woff2", preserveNameIds: LICENSE });
    }
  }
  throw new Error(`${entry.family} に太さ ${weight} の面がカタログにありません`);
}

function check({ entry, weight, file }: (typeof fonts)[number]) {
  const font = create(readFileSync(join(OUTPUT, file))) as Font;
  if (font.familyName !== entry.family) throw new Error(`${file} は ${entry.family} ではありません`);
  if (font["OS/2"].usWeightClass !== weight) throw new Error(`${file} の太さが ${weight} ではありません`);
  if (!font.getName("license", "en")) throw new Error(`${file} にライセンスがありません`);
  const missing = [...TEXT].filter((char) => !font.hasGlyphForCodePoint(char.codePointAt(0)!));
  if (missing.length > 0) {
    throw new Error(`${file} に見本の字がありません: ${missing.join("")}（mise run lp-fonts で作り直す）`);
  }
}

const expected = new Set(fonts.map(({ file }) => file));
const stale = readdirSync(OUTPUT).filter((name) => name.endsWith(".woff2") && !expected.has(name));

if (process.argv.includes("--check")) {
  if (stale.length > 0) throw new Error(`見本で使っていないフォントがあります: ${stale.join(", ")}`);
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
