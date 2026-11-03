// LP（obs.deadlock.jp）。静的なページだけなので、Workers の静的ファイル配信に載せる（wrangler.jsonc）
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://obs.deadlock.jp",
  // /privacy を privacy.html にする（Workers の静的配信は /privacy をそのまま返し、/privacy/ に転送しない）
  build: { format: "file" },
  vite: { plugins: [tailwindcss()] },
});
