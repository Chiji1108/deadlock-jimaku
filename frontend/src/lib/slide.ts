import type { AnimationConfig } from "svelte/animate";
import { cubicOut } from "svelte/easing";

/**
 * 並びが変わった行を縦方向に滑らせる。
 * svelte/animate の flip は大きさの変化も scale で補間するため、partial → final で
 * 行の幅が変わると文字が潰れて見える。字幕では位置だけを補間する。
 */
export function slide(
  _node: Element,
  { from, to }: { from: DOMRect; to: DOMRect },
  { duration = 200 }: { duration?: number } = {},
): AnimationConfig {
  const dy = from.top + from.height - (to.top + to.height);
  return {
    duration,
    easing: cubicOut,
    css: (_t, u) => `transform: translateY(${u * dy}px)`,
  };
}
