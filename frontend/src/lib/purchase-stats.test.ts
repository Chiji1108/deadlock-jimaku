import { expect, test } from "vitest";
import { badge, clock, labels, percent, timing } from "./purchase-stats";

test("小さい割合は小数 1 桁まで出す", () => {
  expect(percent(0.004)).toBe("0.4%");
  expect(percent(0.0996)).toBe("10%");
  expect(percent(0.126)).toBe("13%");
  expect(percent(0.5)).toBe("50%");
});

test("時刻は分と秒で出す", () => {
  expect(clock(65.4)).toBe("1:05");
  expect(clock(1110)).toBe("18:30");
  expect(clock(59.6)).toBe("1:00");
});

test("めったに買われないものと、ほとんどの試合で買われるものにだけバッジを付ける", () => {
  expect(badge(0.019)).toBe("rare");
  expect(badge(0.02)).toBeUndefined();
  expect(badge(0.49)).toBeUndefined();
  expect(badge(0.5)).toBe("staple");
});

test("ラベルの用意のない言語は英語で出す", () => {
  expect(labels("ja").share).toBe("購入率");
  expect(labels("ko").share).toBe("Pick rate");
});

test("平均よりかなり前か後に買ったときだけ、早い・遅いと出す", () => {
  expect(timing(500, 1000)).toBe("early");
  expect(timing(600, 1000)).toBeUndefined();
  expect(timing(1500, 1000)).toBeUndefined();
  expect(timing(1501, 1000)).toBe("late");
});
