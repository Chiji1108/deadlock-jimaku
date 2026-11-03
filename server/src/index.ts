// API（api.deadlock.jp）。パスはアプリごと（/obs）と版（/v1）で区切る（deadlock.jp の下でほかのものを作っても困らない）
import { Hono } from "hono";
import aliasesJa from "../../data/aliases.ja.json" with { type: "json" };

/** 別名はリポジトリの data/ から作る。言語を足したらここに足す */
const ALIASES = new Map<string, Record<string, string[]>>([["ja", aliasesJa]]);

export const app = new Hono().basePath("/obs/v1");

// 別名（話した言葉 → 用語）。アプリは起動時に取りに来て、取れなければ同梱の別名を使う
app.get("/aliases/:lang", (c) => {
  const aliases = ALIASES.get(c.req.param("lang"));
  if (!aliases) return c.notFound();
  // 別名を直したら、1 時間以内に届く
  c.header("Cache-Control", "public, max-age=3600");
  return c.json(aliases);
});

export default app;
