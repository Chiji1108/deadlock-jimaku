import type { Anchor } from "../generated/Anchor";

/** 3×3 の位置（左上から右下へ、操作パネルのボタンの並び順） */
export const ANCHORS: { value: Anchor; label: string }[] = [
  { value: "top-left", label: "左上" },
  { value: "top", label: "上" },
  { value: "top-right", label: "右上" },
  { value: "left", label: "左" },
  { value: "center", label: "中央" },
  { value: "right", label: "右" },
  { value: "bottom-left", label: "左下" },
  { value: "bottom", label: "下" },
  { value: "bottom-right", label: "右下" },
];

/** CSS の place-items（縦・横） */
export function placeItems(anchor: Anchor): string {
  const block = anchor.startsWith("top") ? "start" : anchor.startsWith("bottom") ? "end" : "center";
  const inline = anchor.endsWith("left") ? "start" : anchor.endsWith("right") ? "end" : "center";
  return `${block} ${inline}`;
}
