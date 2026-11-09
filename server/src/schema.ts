// D1 の表（DESIGN.md 17.1）。キーとコードと合言葉は、そのものを保存しない（キーは暗号化、ほかはハッシュ）。
// 列の名前は snake_case（db.ts の casing）。変えたら `pnpm db:generate` で migrations/ に SQL を足す（DESIGN.md 17 章）
import { index, integer, primaryKey, real, sqliteTable, text } from "drizzle-orm/sqlite-core";

/** おすそ分けを有効にした人。同じキーで有効にし直したら同じ行（合言葉もキーで決まる） */
export const issuers = sqliteTable("issuers", {
  id: text().primaryKey(),
  adminTokenHash: text().notNull().unique(),
  keyHash: text().notNull().unique(),
  /** base64(iv || AES-GCM の暗号文) */
  encryptedKey: text().notNull(),
  /** ミリ秒 */
  createdAt: integer().notNull(),
  /** 預けた xAI のキー。形は encryptedKey と同じ。NULL は預けていない（そのコードでは xAI を中継しない） */
  encryptedXaiKey: text(),
});

export const codes = sqliteTable(
  "codes",
  {
    id: text().primaryKey(),
    codeHash: text().notNull().unique(),
    issuerId: text()
      .notNull()
      .references(() => issuers.id, { onDelete: "cascade" }),
    memo: text().notNull().default(""),
    /** ミリ秒。NULL は期限なし */
    expiresAt: integer(),
    revoked: integer({ mode: "boolean" }).notNull().default(false),
    createdAt: integer().notNull(),
    /** 1 日の上限（ドル）。NULL は既定の額（db.ts の DEFAULT_DAILY_LIMIT_USD） */
    dailyLimitUsd: real(),
    /** dayUsd を数えている日（UTC の "2026-11-06"）。今の日の分だけを持ち、日が変わったら上書きする */
    day: text(),
    dayUsd: real().notNull().default(0),
  },
  (t) => [index("codes_issuer").on(t.issuerId)],
);

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
