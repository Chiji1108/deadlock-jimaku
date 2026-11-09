// 表の形（src/schema.ts）から migrations/ の SQL を作る。当てるのは wrangler（`wrangler d1 migrations apply`）
import { defineConfig } from "drizzle-kit";

export default defineConfig({
  dialect: "sqlite",
  schema: "./src/schema.ts",
  out: "./migrations",
  casing: "snake_case",
});
