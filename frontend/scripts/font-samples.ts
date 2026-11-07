// Google Fonts のフォントの見本を、文字をパスにした SVG として frontend/src/control/font-samples に作る（DESIGN.md 6.3）。
// 同時に、カタログの languages の言語の文字がそのフォントにあることを確かめる。
// 引数にフォントのファイルを置いたフォルダを渡すと、GitHub から取らずに使う
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { create, type Font } from "fontkit";
import catalog from "../../crates/core/src/font-catalog.json" with { type: "json" };
import { fontSampleName } from "../src/lib/font-samples.ts";

const OUTPUT = resolve(import.meta.dirname, "../src/control/font-samples");
const ORIGIN = `https://raw.githubusercontent.com/google/fonts/${catalog.commit}`;
const LOCAL = process.argv[2];
/** 1em を分ける数。ダイアログのカードの 28px でも、これで輪郭の違いは見えない */
const UNITS = 128;

const LATIN = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const CYRILLIC = "абвгдежзийклмнопрстуфхцчшщъыьэюяАБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯ";
/** その言語でよく使う文字。全部あるフォントだけを、その言語の Google Fonts に載せられる */
const PROBES: Record<string, string> = {
  ja: "あいうえおかきくけこさしすせそたちつてとアイウエオカキクケコサシスセソー日本語漢字学校先生話聞",
  en: LATIN,
  ru: `${CYRILLIC}ёЁ`,
  "zh-Hans":
    "的一是不了在人有我他这个们中来上大为和国地到以说时要就出会可也你对生能而子那得于着下自之年过发后作里用",
  "zh-Hant":
    "的一是不了在人有我他這個們中來上大為和國地到以說時要就出會可也你對生能而子那得於著下自之年過發後作裡用",
  ko: "가나다라마바사아자차카타파하한국어글자를입니다값뜻",
  th: "กขคงจฉชซญดตถทธนบปผฝพฟภมยรลวศษสหอฮะาำิีึืุูเแโใไ่้๊๋",
  vi: "ăâđêôơưạảấầẩẫậặẹẻẽếềểễệỉịọỏốồổỗộớờởỡợụủứừửữựỳỵỷỹ",
  id: LATIN,
  de: `${LATIN}äöüßÄÖÜ`,
  fr: `${LATIN}éèêëàâçîïôûùœ`,
  es: `${LATIN}ñáéíóúü¿¡`,
  "es-419": `${LATIN}ñáéíóúü¿¡`,
  "pt-PT": `${LATIN}ãõçáéêóúà`,
  "pt-BR": `${LATIN}ãõçáéêóúà`,
  it: `${LATIN}àèéìòù`,
  nl: `${LATIN}ëï`,
  pl: `${LATIN}ąćęłńóśźż`,
  uk: `${CYRILLIC}єіїґЄІЇҐ`,
  cs: `${LATIN}áčďéěíňóřšťúůýž`,
  tr: `${LATIN}çğıİöşü`,
  el: "αβγδεζηθικλμνξοπρστυφχψωάέήίόύώΑΩ",
  hu: `${LATIN}áéíóöőúüű`,
  ro: `${LATIN}ăâîșț`,
  bg: CYRILLIC,
  sv: `${LATIN}åäö`,
  da: `${LATIN}æøå`,
  no: `${LATIN}æøå`,
  fi: `${LATIN}åäö`,
};

type Entry = (typeof catalog.fonts)[number];

async function download(url: string) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${url} を取れません: ${res.status} ${res.statusText}`);
  return Buffer.from(await res.arrayBuffer());
}

async function read(entry: Entry, file: Entry["files"][number]) {
  const local = LOCAL && join(LOCAL, file.name);
  const data =
    local && existsSync(local)
      ? readFileSync(local)
      : await download(`${ORIGIN}/${entry.dir}/${encodeURIComponent(file.name)}`);
  const sha256 = createHash("sha256").update(data).digest("hex");
  if (sha256 !== file.sha256) throw new Error(`${file.name} の SHA-256 が違います`);
  return data;
}

/** core の fontdb と同じく、ファイルの中の名前を読む。カタログの名前と違うと faces.css に出ない */
function checkFamily(data: Buffer, entry: Entry) {
  const font = create(data) as Font;
  const family = font.getName("preferredFamily") ?? font.familyName;
  if (family !== entry.family) throw new Error(`${entry.family} の中の名前は ${family} です`);
}

/** @font-face は太さしか書かないので、同じ太さのファイルがあると core は片方しか配らない */
function checkWeights(files: Buffer[], entry: Entry) {
  const weights = files.map((data) => (create(data) as Font)["OS/2"].usWeightClass);
  if (new Set(weights).size !== weights.length) {
    throw new Error(`${entry.family} の太さが重なっています: ${weights.join(", ")}`);
  }
}

/** 見本の太さは通常の 400。可変フォントの既定は Noto の 100 のように細いことがある */
function regular(data: Buffer) {
  const font = create(data) as Font;
  return "wght" in font.variationAxes ? font.getVariation({ wght: 400 }) : font;
}

function checkLanguages(font: Font, entry: Entry) {
  for (const language of entry.languages) {
    const missing = [...PROBES[language]!].filter(
      (char) => !font.hasGlyphForCodePoint(char.codePointAt(0)!),
    );
    if (missing.length > 0) {
      throw new Error(`${entry.family} に ${language} の文字がありません: ${missing.join("")}`);
    }
  }
}

const COMMANDS: Record<string, string> = {
  moveTo: "m",
  lineTo: "l",
  quadraticCurveTo: "q",
  bezierCurveTo: "c",
};

/** 全部の字を 1 つのパスに、整数の相対座標で書く。z のあとの現在地は、その輪郭の始点に戻る */
function outline(font: Font, text: string, scale: number) {
  const { glyphs, positions } = font.layout(text);
  let at = [0, 0];
  let start = at;
  let pen = 0;
  let d = "";
  glyphs.forEach((glyph, i) => {
    if (glyph.id === 0) throw new Error(`${font.familyName} に「${text}」の字がありません`);
    const x = pen + positions[i]!.xOffset;
    for (const { command, args } of glyph.path.commands) {
      if (command === "closePath") {
        d += "z";
        at = start;
        continue;
      }
      const points = [];
      for (let p = 0; p < args.length; p += 2) {
        points.push([Math.round((args[p]! + x) * scale), Math.round(-args[p + 1]! * scale)]);
      }
      const deltas = points.flatMap(([px, py]) => [px! - at[0]!, py! - at[1]!]);
      d += COMMANDS[command]! + deltas.join(" ").replaceAll(" -", "-");
      at = points.at(-1)!;
      if (command === "moveTo") start = at;
    }
    pen += positions[i]!.xAdvance;
  });
  return { d, width: Math.round(pen * scale) };
}

function svg(font: Font, text: string) {
  const { d, width } = outline(font, text, UNITS / font.unitsPerEm);
  const top = Math.round(UNITS * 1.0);
  const height = Math.round(UNITS * 1.3);
  const em = ((width / height) * 1.3).toFixed(2);
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 -${top} ${width} ${height}" width="${em}em" height="1.3em" fill="currentColor"><path d="${d}"/></svg>\n`;
}

const samples = new Map<string, string>();
for (const entry of catalog.fonts) {
  const files = await Promise.all(entry.files.map((file) => read(entry, file)));
  for (const data of files) checkFamily(data, entry);
  checkWeights(files, entry);
  const font = regular(files[0]!);
  checkLanguages(font, entry);
  const out = svg(font, entry.sample);
  samples.set(fontSampleName(entry.family), out);
  console.log(entry.family, out.length);
}

// 1 つでも失敗したら、今の見本を残す
rmSync(OUTPUT, { recursive: true, force: true });
mkdirSync(OUTPUT);
for (const [name, out] of samples) writeFileSync(join(OUTPUT, name), out);
