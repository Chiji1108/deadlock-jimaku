// 更新の内容（リポジトリの CHANGELOG.md）。書く場所は 1 つにして、アプリはそれを読む
import text from "../../../CHANGELOG.md?raw";

export type Release = { version: string; items: string[] };

/** `## 版` の見出しと `- ` の箇条書きを読む。新しい版が先 */
export function parse(markdown: string): Release[] {
  const releases: Release[] = [];
  for (const line of markdown.split("\n")) {
    if (line.startsWith("## ")) releases.push({ version: line.slice(3).trim(), items: [] });
    else if (line.startsWith("- ")) releases.at(-1)?.items.push(line.slice(2).trim());
  }
  return releases;
}

export const RELEASES = parse(text);

/** プレリリースの印を除いた版（`0.6.0-rc.11` なら `0.6.0`）。更新の内容は、この版ごとに書く */
export const releaseOf = (version: string) => version.split("-")[0]!;

const numbers = (version: string) => version.split(".").map(Number);

/** a が b より新しい */
const newer = (a: string, b: string) => {
  const [x, y] = [numbers(a), numbers(b)];
  const at = x.findIndex((n, i) => n !== (y[i] ?? 0));
  return at !== -1 && x[at]! > (y[at] ?? 0);
};

/** まだ見ていない版の内容。`seen` がなければ（この仕組みができる前から使っている人）、今の版まで全部 */
export const unseen = (releases: Release[], seen: string | undefined, current: string) =>
  releases.filter(
    (r) => !newer(r.version, releaseOf(current)) && (seen === undefined || newer(r.version, seen)),
  );
