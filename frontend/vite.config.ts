import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

// axum（crates/core の DEFAULT_PORT）
const core = "127.0.0.1:4510";

export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  // shadcn-svelte の $lib（/ から始めるとプロジェクトのルートからの位置）
  resolve: { alias: { $lib: "/src/lib" } },
  // OBS に貼る URL は、開発中（Vite から開いたとき）も axum のもの
  define: { "import.meta.env.CORE_ORIGIN": JSON.stringify(`http://${core}`) },
  build: {
    // /assets はゲームアイコンの配信に使うので、ビルド成果物は /_app に置く
    assetsDir: "_app",
    rollupOptions: {
      input: {
        control: "control.html",
        chrome: "chrome.html",
        subtitles: "overlay/subtitles.html",
        rank: "overlay/rank.html",
        items: "overlay/items.html",
      },
    },
  },
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      "/ws": { target: `ws://${core}`, ws: true },
      // Host をそのまま渡す（文字列の形だと書き換わり、axum の Origin の確かめに通らない）
      "/api": { target: `http://${core}`, ws: true },
      "/assets": `http://${core}`,
    },
  },
});
