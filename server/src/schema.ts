// D1 の表（DESIGN.md 17 章）。キーとコードは、そのものを保存しない（キーは暗号化、コードはハッシュ）。
// 列の名前は snake_case（db.ts の casing）。変えたら `pnpm db:generate` で migrations/ に SQL を足す（DESIGN.md 17 章）
import { index, integer, primaryKey, real, sqliteTable, text } from "drizzle-orm/sqlite-core";

/** ベータのコード（DESIGN.md 17.2）。アプリが自動でもらい、これで使う人を見分ける */
export const codes = sqliteTable("codes", {
    id: text().primaryKey(),
    codeHash: text().notNull().unique(),
    revoked: integer({ mode: "boolean" }).notNull().default(false),
    createdAt: integer().notNull(),
    /** 1 日の上限（ドル）。NULL は既定の額（db.ts の CODE_DAILY_USD） */
    dailyLimitUsd: real(),
    /** dayUsd を数えている日（UTC の "2026-11-06"）。今の日の分だけを持ち、日が変わったら上書きする */
    day: text(),
    dayUsd: real().notNull().default(0),
});

/** コードで使われた額（月ごと。月は UTC の "2026-11"） */
export const codeUsage = sqliteTable(
  "code_usage",
  {
    codeId: text()
      .notNull()
      .references(() => codes.id, { onDelete: "cascade" }),
    month: text().notNull(),
    usd: real().notNull().default(0),
  },
  (t) => [primaryKey({ columns: [t.codeId, t.month] })],
);

/** ベータのコードを出した数（DESIGN.md 17.2）。同じ所に出しすぎないために数える。古い日の行は使わない */
export const betaGrants = sqliteTable(
  "beta_grants",
  {
    /** UTC の "2026-11-11" */
    day: text().notNull(),
    /** IP アドレス（IPv6 は先頭の 64 ビット）と日付のハッシュ（IP アドレスそのものは保存しない）。"*" はその日の合計 */
    place: text().notNull(),
    count: integer().notNull().default(0),
  },
  (t) => [primaryKey({ columns: [t.day, t.place] })],
);

/** ベータの全部のコードが、その日（UTC）に使った額。上限を、1 行の読み書きで確かめるために持つ */
export const betaSpend = sqliteTable("beta_spend", {
  day: text().primaryKey(),
  usd: real().notNull().default(0),
});

/**
 * ベータで使う開発者のキー（DESIGN.md 17.3）。会社ごとに複数置ける（回数の上限に当たったら、次のキーに回す）。
 * 管理画面から足す。キーそのものは保存しない（暗号化）
 */
export const betaKeys = sqliteTable(
  "beta_keys",
  {
    id: text().primaryKey(),
    provider: text({ enum: ["qwen", "openrouter"] }).notNull(),
    /** base64(iv || AES-GCM の暗号文) */
    encryptedKey: text().notNull(),
    /** 見分けるための、キーの終わりの 4 文字 */
    tail: text().notNull(),
    memo: text().notNull().default(""),
    enabled: integer({ mode: "boolean" }).notNull().default(true),
    /** 外に断られた回数と、最後に断られたとき（ミリ秒）とその理由（HTTP の番号） */
    failures: integer().notNull().default(0),
    failedAt: integer(),
    failedStatus: integer(),
    createdAt: integer().notNull(),
  },
  (t) => [index("beta_keys_provider").on(t.provider)],
);

/** ベータの設定（管理画面から変える）。`closed` が "1" なら、コードを出さず、中継もしない */
export const betaSettings = sqliteTable("beta_settings", {
  name: text().primaryKey(),
  value: text().notNull(),
});
