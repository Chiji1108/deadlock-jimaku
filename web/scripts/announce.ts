// 新しい版の告知の画像（Discord などに貼る 1 枚）を作る。web/announce/index.html を Chrome で開いて PNG にする。
//
// usage: mise run announce -- --version 0.12.0 --title "[買ったアイテム]の|表示を追加" \
//          --point "ゲームでアイテムを買うと、配信に小さく出る" --point "…" \
//          --image 画面の絵.png [--image もう 1 枚.png] [--out target/announce.png]
//   --title  `[ ]` で囲んだところに黄色の帯を引き、`|` で改行する
//   --image  右に並べる絵（1〜2 枚）。暗い板に載せるので、アプリの画面（暗い地）を切り出したものが合う
//   出るのは 2400×1350 の PNG（Chrome が要る）。見出しのフォント（Noto Sans JP）は、初めの 1 回だけ GitHub から取って
//   target/announce/ に置く（LP に置いてあるものは、LP の見出しに出る字しか入っていない）
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import catalog from "../../crates/core/src/font-catalog.json" with { type: "json" };

const CHROME =
  process.platform === "darwin"
    ? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    : process.platform === "win32"
      ? "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe"
      : "google-chrome";

const { values } = parseArgs({
  options: {
    version: { type: "string" },
    title: { type: "string" },
    point: { type: "string", multiple: true, default: [] },
    image: { type: "string", multiple: true, default: [] },
    out: { type: "string", default: "target/announce.png" },
  },
});
if (!values.version || !values.title) throw new Error("--version と --title が要ります");

/** LP の大きい見出しと同じフォント（全部の字が入った元のファイル）。カタログの google/fonts のコミットから取り、SHA-256 で確かめる */
async function displayFont(): Promise<string> {
  const entry = catalog.fonts.find((font) => font.family === "Noto Sans JP")!;
  const file = entry.files[0]!;
  const path = resolve(import.meta.dirname, "../../target/announce", file.name);
  if (!existsSync(path)) {
    const url = `https://raw.githubusercontent.com/google/fonts/${catalog.commit}/${entry.dir}/${encodeURIComponent(file.name)}`;
    const res = await fetch(url);
    if (!res.ok) throw new Error(`${url} を取れません: ${res.status} ${res.statusText}`);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, Buffer.from(await res.arrayBuffer()));
  }
  if (createHash("sha256").update(readFileSync(path)).digest("hex") !== file.sha256) {
    throw new Error(`${path} の SHA-256 が違います（消して、やり直してください）`);
  }
  return path;
}

const page = pathToFileURL(resolve(import.meta.dirname, "../announce/index.html"));
page.searchParams.set("font", pathToFileURL(await displayFont()).href);
page.searchParams.set("version", values.version);
page.searchParams.set("title", values.title);
for (const point of values.point) page.searchParams.append("point", point);
for (const image of values.image) {
  if (!existsSync(image)) throw new Error(`絵がありません: ${image}`);
  page.searchParams.append("image", pathToFileURL(resolve(image)).href);
}

const out = resolve(values.out);
mkdirSync(dirname(out), { recursive: true });
execFileSync(
  CHROME,
  [
    "--headless",
    "--hide-scrollbars",
    // 絵とフォントを、この頁（file://）から読む
    "--allow-file-access-from-files",
    "--window-size=1200,675",
    "--force-device-scale-factor=2",
    // フォントと絵を読み終えるまで待つ
    "--virtual-time-budget=5000",
    `--screenshot=${out}`,
    page.href,
  ],
  { stdio: "ignore" },
);
console.log(out);
