import { expect, test } from "vitest";
import { validCode } from "./CodeFields.svelte";

const NOW = 1000;
const LIMIT = 5;

test("新しいコードは過去の期限を受け付けない", () => {
  expect(validCode(NOW - 1, LIMIT, NOW)).toBe(false);
  expect(validCode(NOW + 1, LIMIT, NOW)).toBe(true);
  expect(validCode(null, LIMIT, NOW)).toBe(true);
});

test("保存済みの期限のままなら、過ぎていても保存できる", () => {
  expect(validCode(NOW - 1, LIMIT, NOW, NOW - 1)).toBe(true);
});

test("期限を変えたら、過去の日時を拒否する", () => {
  expect(validCode(NOW - 2, LIMIT, NOW, NOW - 1)).toBe(false);
  expect(validCode(NOW + 1, LIMIT, NOW, NOW - 1)).toBe(true);
  expect(validCode(null, LIMIT, NOW, NOW - 1)).toBe(true);
});

test("1 日の上限は正の数だけ", () => {
  expect(validCode(NOW + 1, 0, NOW)).toBe(false);
  expect(validCode(NOW + 1, -1, NOW)).toBe(false);
  expect(validCode(NOW + 1, null, NOW)).toBe(false);
  expect(validCode(NOW - 1, null, NOW, NOW - 1)).toBe(false);
});
