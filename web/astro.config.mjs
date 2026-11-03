// LP（obs.deadlock.jp）。静的なページだけなので、Workers の静的ファイル配信に載せる（wrangler.jsonc）
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://obs.deadlock.jp",
  vite: { plugins: [tailwindcss()] },
});
