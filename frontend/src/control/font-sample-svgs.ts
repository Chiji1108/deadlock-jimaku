// 見本の SVG は合わせて 350 KB あるので、ダイアログを初めて開くまで読まない（panel.loadFontSamples）

/** リポジトリの中の、文字をパスにした SVG（scripts/font-samples.ts）。文字の色で描く */
export const SAMPLES = import.meta.glob<string>("./font-samples/*.svg", {
  eager: true,
  query: "?raw",
  import: "default",
});
