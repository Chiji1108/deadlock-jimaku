// 配布物に入る第三者のソフトウェアとフォントのライセンス表示を notices/third-party.txt に作る（DESIGN.md 13.7）。
// --check は書き込まず、ファイルが古ければ失敗する
import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { build, type Plugin } from "vite";

type Package = { name: string; version: string; license: string; dir: string; authors: string[] };

const ROOT = resolve(import.meta.dirname, "../..");
const FRONTEND = join(ROOT, "frontend");
const OUTPUT = join(ROOT, "notices/third-party.txt");
// build.yml で配るターゲット。cfg で分かれる依存を漏らさないよう、手元の OS によらず全部を解決する
const TARGETS = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "x86_64-pc-windows-msvc",
  "x86_64-unknown-linux-gnu",
];
const LICENSE_FILE = /^(licen[sc]e|copying|copyright|unlicense|third[-_]party|mit$|apache)/i;
const NOTICE_FILE = /^notice/i;
const SKIPPED_EXTENSION = /\.(rs|spdx|c|h|cmake|[cm]?[jt]s|map|json|css|svelte|svg|py)$/;
// features で vendored を選ばないので、ビルドされない（システムの libdbus を使う）
const UNBUILT = /\/libdbus-sys-[^/]+\/vendor\//;
function cargo(args: string[]) {
  return execFileSync("cargo", ["--locked", ...args], {
    cwd: ROOT,
    encoding: "utf8",
    maxBuffer: 1 << 28,
  });
}

// リンクされる crate だけを選ぶ。dev-dependencies、build-dependencies、proc-macro は成果物に入らない。
// cargo tree はターゲットごとに features を解決するので、1 回で全ターゲットを渡さない
function rustPackages(): Package[] {
  const linked = new Set(
    TARGETS.flatMap((target) =>
      cargo([
        "tree",
        "-p=deadlock-jimaku",
        "--edges=normal,no-proc-macro",
        `--target=${target}`,
        "--prefix=none",
        "--format={p}",
      ])
        .split("\n")
        .map((line) => line.replace(/ \(.*/, "")),
    ),
  );
  const { packages } = JSON.parse(
    cargo([
      "metadata",
      "--format-version=1",
      ...TARGETS.map((target) => `--filter-platform=${target}`),
    ]),
  ) as {
    packages: {
      name: string;
      version: string;
      license: string | null;
      manifest_path: string;
      authors: string[];
      source: string | null;
    }[];
  };
  return packages
    .filter((p) => p.source && linked.has(`${p.name} v${p.version}`))
    .map((p) => ({
      name: p.name,
      version: p.version,
      license: p.license ?? "（記載なし）",
      dir: dirname(p.manifest_path),
      authors: p.authors,
    }));
}

// frontend はほぼ全部が devDependencies なので、package.json ではなく実際のバンドルから取る。
// Tailwind が取り込む CSS はモジュールにならないので、@import からも拾う
async function frontendPackages(): Promise<Package[]> {
  const dirs = new Set<string>();
  const collect = (file: string) => {
    const dir = /^.*\/node_modules\/(@[^/]+\/)?[^/]+(?=\/|$)/.exec(resolve(FRONTEND, file))?.[0];
    if (dir) dirs.add(dir);
  };
  const collector: Plugin = {
    name: "collect-packages",
    transform: {
      order: "pre",
      handler(code, id) {
        if (!id.endsWith(".css") || id.includes("/node_modules/")) return;
        for (const [, name] of code.matchAll(/@import "(?!\.)((?:@[^/"]+\/)?[^/"]+)/g)) {
          collect(realpathSync(join(FRONTEND, "node_modules", name)));
        }
      },
    },
    generateBundle(_, bundle) {
      for (const item of Object.values(bundle)) {
        if (item.type === "asset") item.originalFileNames.forEach(collect);
        else
          Object.entries(item.modules)
            .filter(([, module]) => module.renderedLength > 0)
            .forEach(([id]) => collect(id));
      }
    },
  };
  await build({
    root: FRONTEND,
    logLevel: "silent",
    build: { write: false },
    plugins: [collector],
  });
  return [...dirs].map((dir) => {
    const json = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    const author = typeof json.author === "string" ? json.author : json.author?.name;
    return {
      name: json.name,
      version: json.version,
      license: json.license ?? "（記載なし）",
      dir,
      authors: author ? [author] : [],
    };
  });
}

// 取り込んだコードの LICENSE はサブフォルダにあるので、下の階層まで見る（ring、regex-syntax など）
function textFiles(dir: string, pattern: RegExp) {
  return readdirSync(dir, { recursive: true, encoding: "utf8" })
    .filter((path) => pattern.test(basename(path)) && !SKIPPED_EXTENSION.test(path))
    .filter((path) => !UNBUILT.test(join(dir, path)) && statSync(join(dir, path)).isFile())
    .toSorted()
    .map((path) => readFileSync(join(dir, path), "utf8").replaceAll("\r\n", "\n").trim());
}

function isCopyright(line: string) {
  return (
    /^\W*(copyright\b.*(\d{4}|\(c\)|©)|©|\(c\)\s*\d)/i.test(line) &&
    !/\[yyyy\]|\{yyyy\}|<year>|\[year\]|\{year\}/i.test(line)
  );
}

// 記号や空白の違い、Apache の末尾の「適用のしかた」（本文ではない）は同じ本文として扱う
function fingerprint(body: string) {
  return body
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .replaceAll("https", "http")
    .replace(/^\W*(the )?mit license\W*(mit\W*)?/, "")
    .replace(/appendix how to apply the apache license.*?limitations under the license/, "")
    .replace(/end of terms and conditions/, "")
    .trim();
}

function render(sections: { title: string; packages: Package[] }[]) {
  // 指紋 → 本文。同じ本文は 1 度だけ書き、パッケージからは番号で指す
  const texts = new Map<string, { body: string; verbatim: string; users: Set<string> }>();
  const notices: string[] = [];
  const missing: string[] = [];
  const lines: string[] = [];

  const entries = (pkg: Package) => {
    const files = textFiles(pkg.dir, LICENSE_FILE);
    const label = `${pkg.name} ${pkg.version}`;
    if (files.length === 0) missing.push(`${label}  ${pkg.license}`);
    const fingerprints = files.map((text) => {
      const body = text
        .split("\n")
        .filter((line) => !isCopyright(line))
        .join("\n")
        .replace(/\n{3,}/g, "\n\n")
        .trim();
      const key = fingerprint(body);
      const entry = texts.get(key) ?? { body, verbatim: text, users: new Set() };
      entry.users.add(label);
      texts.set(key, entry);
      return key;
    });
    const copyrights = [
      ...new Set(
        files.flatMap((text) =>
          text
            .split("\n")
            .filter(isCopyright)
            .map((line) => line.replace(/^[\s/#*]+/, "").trimEnd()),
        ),
      ),
    ];
    for (const text of textFiles(pkg.dir, NOTICE_FILE)) notices.push(`--- ${label}\n\n${text}`);
    return { label, license: pkg.license, fingerprints, copyrights, authors: pkg.authors };
  };

  const rendered = sections.map(({ title, packages }) => ({
    title,
    entries: packages
      .toSorted((a, b) => cmp(a.name, b.name) || cmp(a.version, b.version))
      .map(entries),
  }));

  const order = [...texts.entries()]
    .toSorted(([ka, a], [kb, b]) => b.users.size - a.users.size || cmp(ka, kb))
    .map(([key]) => key);

  lines.push(
    "デドロ字幕の配布物に含まれる第三者のソフトウェアとフォントのライセンス表示です。",
    "mise run notices で生成します。手で編集しないでください。",
    "複数のライセンスから選べるものは、そのうち 1 つのもとで利用しています。[n] はパッケージに含まれるライセンスファイルの本文で、末尾にあります。",
    "",
  );
  for (const { title, entries: list } of rendered) {
    lines.push(`=== ${title}（${list.length} 件）===`, "");
    for (const entry of list) {
      const refs = entry.fingerprints.map((key) => `[${order.indexOf(key) + 1}]`);
      lines.push(`${entry.label}  ${entry.license} ${[...new Set(refs)].join("")}`.trimEnd());
      const credits =
        entry.copyrights.length > 0
          ? entry.copyrights
          : entry.authors.map((author) => `Author: ${author}`);
      lines.push(...credits.map((credit) => `  ${credit}`));
    }
    lines.push("");
  }
  if (missing.length > 0) {
    lines.push(
      `=== パッケージにライセンスファイルがないもの（${missing.length} 件）===`,
      "",
      "ライセンスはパッケージのメタデータの記載です。",
      "",
      ...missing,
      "",
    );
  }
  if (notices.length > 0) lines.push("=== NOTICE ===", "", ...notices, "");
  lines.push("=== ライセンス本文 ===", "");
  order.forEach((key, index) => {
    const { body, verbatim, users } = texts.get(key)!;
    lines.push(`--- [${index + 1}]`, "", users.size > 1 ? body : verbatim, "");
  });
  return `${lines.join("\n").trim()}\n`;
}

function cmp(a: string, b: string) {
  return a < b ? -1 : a > b ? 1 : 0;
}

const output = render([
  { title: "フロントエンドとフォント", packages: await frontendPackages() },
  { title: "Rust の crate", packages: rustPackages() },
]);

if (process.argv.includes("--check")) {
  if (readFileSync(OUTPUT, "utf8") !== output) {
    console.error("notices/third-party.txt が古い。mise run notices で作り直す");
    process.exit(1);
  }
} else {
  writeFileSync(OUTPUT, output);
}
