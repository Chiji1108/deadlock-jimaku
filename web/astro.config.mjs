// LP（jimaku.deadlock.jp）。Workers の静的ファイル配信に載せる（wrangler.jsonc）
import svelte from "@astrojs/svelte";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://jimaku.deadlock.jp",
  // /privacy を privacy.html にする（Workers の静的配信は /privacy をそのまま返し、/privacy/ に転送しない）
  build: { format: "file" },
  // 字幕とランクは、アプリの部品（frontend/src/lib）をそのまま使う
  integrations: [svelte()],
  vite: { plugins: [tailwindcss()] },
});
