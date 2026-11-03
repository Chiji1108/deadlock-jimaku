// API（api.jimaku.deadlock.jp）。今は何も返さない（ギフトコードをここに作る。DESIGN.md 17 章）。パスは版（/v1）で区切る
import { Hono } from "hono";

export const app = new Hono().basePath("/v1");

export default app;
