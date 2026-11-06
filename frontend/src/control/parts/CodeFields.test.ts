import { expect, test } from "vitest";
import { validCode } from "./CodeFields.svelte";

const NOW = 1000;

test("新しいコードは過去の期限を受け付けない", () => {
  expect(validCode(NOW - 1, null, NOW)).toBe(false);
  expect(validCode(NOW + 1, null, NOW)).toBe(true);
  expect(validCode(null, null, NOW)).toBe(true);
});

test("保存済みの期限のままなら、過ぎていても保存できる", () => {
  expect(validCode(NOW - 1, 5, NOW, NOW - 1)).toBe(true);
});

test("期限を変えたら、過去の日時を拒否する", () => {
  expect(validCode(NOW - 2, null, NOW, NOW - 1)).toBe(false);
  expect(validCode(NOW + 1, null, NOW, NOW - 1)).toBe(true);
  expect(validCode(null, null, NOW, NOW - 1)).toBe(true);
});

test("1 日の上限は正の数か空欄", () => {
  expect(validCode(NOW - 1, 0, NOW, NOW - 1)).toBe(false);
});
