// 選んだ色から字幕の色を求める。OBS 30 のブラウザソース（Chromium 103）は color-mix() や
// oklch() を読めないので、JS で計算して CSS 変数に入れる
type Rgb = [number, number, number];

const each = ([x, y, z]: Rgb, f: (v: number) => number): Rgb => [f(x), f(y), f(z)];
const channel = (hex: string, at: number) => parseInt(hex.slice(at, at + 2), 16) / 255;
const parse = (hex: string): Rgb => [channel(hex, 1), channel(hex, 3), channel(hex, 5)];
const format = (rgb: Rgb) =>
  `#${rgb
    .map((v) =>
      Math.round(Math.min(1, Math.max(0, v)) * 255)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;

const toLinear = (v: number) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
const fromLinear = (v: number) => (v <= 0.0031308 ? v * 12.92 : 1.055 * v ** (1 / 2.4) - 0.055);
const linear = (hex: string) => each(parse(hex), toLinear);

/** 相対輝度（WCAG） */
export function luminance(hex: string) {
  const [r, g, b] = linear(hex);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** a から b へ t だけ寄せた色（sRGB で混ぜる。CSS の color-mix(in srgb) と同じ） */
export function mix(a: string, b: string, t: number) {
  const [[r1, g1, b1], [r2, g2, b2]] = [parse(a), parse(b)];
  return format([r1 + (r2 - r1) * t, g1 + (g2 - g1) * t, b1 + (b2 - b1) * t]);
}

// OKLab と線形 sRGB の変換（https://bottosson.github.io/posts/oklab/）
function toOklab(hex: string): Rgb {
  const [r, g, b] = linear(hex);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}
function fromOklab([L, a, b]: Rgb): Rgb {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

const inGamut = (rgb: Rgb) => rgb.every((v) => v >= -1e-4 && v <= 1 + 1e-4);

/**
 * OKLCH の明るさを反転した色。白と黒に対するコントラスト比（WCAG）が入れ替わる明るさに移す
 * （1 - L だと暗い側に寄りすぎ、色味が残らない）。色相は変えず、彩度は sRGB に収まるところまで落とす
 */
export function invertLightness(hex: string) {
  const [L, a, b] = toOklab(hex);
  // 無彩色では輝度が L の 3 乗。白と黒へのコントラスト比 (Y + 0.05) / 0.05 と 1.05 / (Y + 0.05) を入れ替える
  const inverted = Math.cbrt(Math.max(0, (1.05 * 0.05) / (L ** 3 + 0.05) - 0.05));
  const at = (k: number) => fromOklab([inverted, a * k, b * k]);
  let [low, high] = [0, 1];
  if (inGamut(at(1))) low = 1;
  else
    for (let i = 0; i < 20; i++) {
      const k = (low + high) / 2;
      if (inGamut(at(k))) low = k;
      else high = k;
    }
  return format(each(at(low), fromLinear));
}
