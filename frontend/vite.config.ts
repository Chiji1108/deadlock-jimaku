import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// axum（crates/core の DEFAULT_PORT）
const core = "127.0.0.1:4510";

export default defineConfig({
  plugins: [svelte()],
  build: {
    // /assets はゲームアイコンの配信に使うので、ビルド成果物は /_app に置く
    assetsDir: "_app",
    rollupOptions: {
      input: {
        control: "control.html",
        subtitles: "overlay/subtitles.html",
        rank: "overlay/rank.html",
      },
    },
  },
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      "/ws": { target: `ws://${core}`, ws: true },
      "/api": `http://${core}`,
      "/assets": `http://${core}`,
    },
  },
});
