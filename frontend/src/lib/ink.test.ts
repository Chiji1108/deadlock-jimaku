import { expect, test } from "vitest";
import { alphaSpan } from "./ink";

/** 1 行の画像。数字は不透明度 */
const row = (...alphas: number[]) =>
  new Uint8ClampedArray(alphas.flatMap((alpha) => [255, 255, 255, alpha]));

test("透明な縁を除いた左右の端を、幅との比で返す", () => {
  expect(alphaSpan(row(0, 0, 255, 128, 0), 5)).toEqual([2 / 5, 4 / 5]);
});

test("かすかな画素は透明とみなし、全部透明なら返さない", () => {
  expect(alphaSpan(row(8, 255, 8, 8), 4)).toEqual([1 / 4, 2 / 4]);
  expect(alphaSpan(row(0, 8, 0), 3)).toBeUndefined();
});

test("2 行以上は、どの行の端も含める", () => {
  const data = new Uint8ClampedArray([...row(0, 255, 0, 0), ...row(0, 0, 0, 255)]);
  expect(alphaSpan(data, 4)).toEqual([1 / 4, 1]);
});
