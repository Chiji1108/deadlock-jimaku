// 箱ではなく、実際に色の付く範囲（インク）で余白を測る。ランクのアイコンは周りに透明な縁があり、
// 縁の幅はティアごとに違う。筆記体や斜体のフォントは字の形が送り幅の外にはみ出す

/** 透明とみなす不透明度（0〜255）。webp の圧縮で縁に残るかすかな画素は数えない */
const CLEAR = 16;

/** 透明でない画素の行の左右の端（幅との比）。全部透明なら undefined */
export function alphaSpan(data: Uint8ClampedArray, width: number): [number, number] | undefined {
  let left = width;
  let right = -1;
  for (let i = 3; i < data.length; i += 4) {
    if (data[i]! <= CLEAR) continue;
    const x = ((i - 3) / 4) % width;
    left = Math.min(left, x);
    right = Math.max(right, x);
  }
  return right < 0 ? undefined : [left / width, (right + 1) / width];
}

const imageSpans = new Map<string, [number, number]>();

/** 画像のインクの左右の端（画像の幅との比）。まだ読めていない画像は undefined、画素を読めない画像（別のオリジン）は全体 */
function imageSpan(img: HTMLImageElement) {
  const { currentSrc: src, naturalWidth: width, naturalHeight: height } = img;
  if (!img.complete || width === 0) return undefined;
  let span = imageSpans.get(src);
  if (!span) {
    const ctx = new OffscreenCanvas(width, height).getContext("2d")!;
    ctx.drawImage(img, 0, 0);
    try {
      span = alphaSpan(ctx.getImageData(0, 0, width, height).data, width) ?? [0, 1];
    } catch {
      span = [0, 1];
    }
    imageSpans.set(src, span);
  }
  return span;
}

let measurer: OffscreenCanvasRenderingContext2D | undefined;

/** 1 行の文字のインクの左右の端（文字の始まりからの px）。縁取りと影は含めない（text-style.ts の --edge） */
function textSpan(p: HTMLElement): [number, number] {
  const style = getComputedStyle(p);
  measurer ??= new OffscreenCanvas(1, 1).getContext("2d")!;
  measurer.font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
  measurer.letterSpacing = style.letterSpacing === "normal" ? "0px" : style.letterSpacing;
  const metrics = measurer.measureText(p.textContent ?? "");
  return [-metrics.actualBoundingBoxLeft, metrics.actualBoundingBoxRight];
}

/** 画面での要素のインクの左右の端。画像は object-fit: contain で中央に置いたときの位置 */
function inkOf(el: HTMLElement, scale: number): [number, number] {
  const box = el.getBoundingClientRect();
  if (el instanceof HTMLImageElement) {
    const span = imageSpan(el);
    if (!span) return [box.left, box.right];
    const drawn =
      Math.min(box.width / el.naturalWidth, box.height / el.naturalHeight) * el.naturalWidth;
    const start = box.left + (box.width - drawn) / 2;
    return [start + span[0] * drawn, start + span[1] * drawn];
  }
  if (el instanceof HTMLParagraphElement) {
    const [left, right] = textSpan(el);
    return [box.left + left * scale, box.left + right * scale];
  }
  return [box.left, box.right];
}

/**
 * 中身の箱の端からインクの端までの空き（CSS の px）。lead は始まりの側、trail は終わりの側で、
 * インクが箱の外にはみ出すと負になる。`parts` は中身のうち、端に来うる要素
 */
export function inkInsets(container: HTMLElement, parts: HTMLElement[]) {
  const box = container.getBoundingClientRect();
  // 変形（出てくるときの拡大）や zoom の中でも CSS の px で返す
  const scale = box.width / container.offsetWidth || 1;
  const style = getComputedStyle(container);
  const start = box.left + parseFloat(style.paddingLeft) * scale;
  const end = box.right - parseFloat(style.paddingRight) * scale;
  const inks = parts.map((part) => inkOf(part, scale));
  if (inks.length === 0) return { lead: 0, trail: 0 };
  return {
    lead: (Math.min(...inks.map(([left]) => left)) - start) / scale,
    trail: (end - Math.max(...inks.map(([, right]) => right))) / scale,
  };
}

/**
 * 中身が変わるたびに測り直す（Svelte の attach）。文字やフォントが変われば中身の箱の大きさが変わり、
 * 画像は大きさが決まっているので読み終えたときに測る。余白を変えても中身の箱は変わらないので、繰り返さない
 */
export const watchInk =
  (selector: string, onChange: (insets: { lead: number; trail: number }) => void) =>
  (container: HTMLElement) => {
    const measure = () =>
      onChange(inkInsets(container, [...container.querySelectorAll<HTMLElement>(selector)]));
    const observer = new ResizeObserver(measure);
    observer.observe(container);
    container.addEventListener("load", measure, true);
    return () => {
      observer.disconnect();
      container.removeEventListener("load", measure, true);
    };
  };
