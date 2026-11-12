import { expect, test } from "vitest";
import { RELEASES, parse, releaseOf, unseen } from "./changelog";

const releases = parse(
  "# 題\n\n説明\n\n## 0.7.0\n\n- 新しい\n\n## 0.6.1\n\n- 直した\n- もう 1 つ\n\n## 0.6.0\n\n- 最初\n",
);

test("版ごとの箇条書きを読む", () => {
  expect(releases).toEqual([
    { version: "0.7.0", items: ["新しい"] },
    { version: "0.6.1", items: ["直した", "もう 1 つ"] },
    { version: "0.6.0", items: ["最初"] },
  ]);
});

test("見ていない版だけを、今の版まで出す", () => {
  const versions = (seen: string | undefined, current: string) =>
    unseen(releases, seen, current).map((r) => r.version);
  // 前から使っている人には、今の版まで全部
  expect(versions(undefined, "0.6.1")).toEqual(["0.6.1", "0.6.0"]);
  // 何版か飛ばした人には、見ていない分をまとめて
  expect(versions("0.6.0", "0.7.0")).toEqual(["0.7.0", "0.6.1"]);
  expect(versions("0.7.0", "0.7.0")).toEqual([]);
  // プレリリースは、本リリースの版の内容を 1 回だけ
  expect(versions("0.6.1", "0.7.0-rc.1")).toEqual(["0.7.0"]);
  expect(versions(releaseOf("0.7.0-rc.1"), "0.7.0-rc.2")).toEqual([]);
  // 桁が増えても数として比べる
  expect(unseen(parse("## 0.10.0\n- a\n"), "0.9.0", "0.10.0")).toHaveLength(1);
});

test("リポジトリの更新の内容は、どの版にも中身がある", () => {
  expect(RELEASES.length).toBeGreaterThan(0);
  for (const r of RELEASES) {
    expect(r.version).toMatch(/^\d+\.\d+\.\d+$/);
    expect(r.items.length).toBeGreaterThan(0);
  }
});
