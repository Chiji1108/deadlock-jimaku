-- おすそ分け（人のキーで中継するギフトコード）をやめる。預かったキーと、おすそ分けのコード・その使用額を消し、
-- ベータのコードとその使用額だけを残す。codes から、持ち主・メモ・期限の列をなくす。
-- D1 は外部キーを切れない（表を消すと、連鎖でつながる行も消える）ので、残す行を新しい表に写してから、古い表を消す
CREATE TABLE `codes_new` (
	`id` text PRIMARY KEY NOT NULL,
	`code_hash` text NOT NULL,
	`revoked` integer DEFAULT false NOT NULL,
	`created_at` integer NOT NULL,
	`daily_limit_usd` real,
	`day` text,
	`day_usd` real DEFAULT 0 NOT NULL
);
--> statement-breakpoint
INSERT INTO `codes_new`("id", "code_hash", "revoked", "created_at", "daily_limit_usd", "day", "day_usd") SELECT "id", "code_hash", "revoked", "created_at", "daily_limit_usd", "day", "day_usd" FROM `codes` WHERE "issuer_id" = 'beta';
--> statement-breakpoint
CREATE TABLE `code_usage_new` (
	`code_id` text NOT NULL,
	`month` text NOT NULL,
	`usd` real DEFAULT 0 NOT NULL,
	PRIMARY KEY(`code_id`, `month`),
	FOREIGN KEY (`code_id`) REFERENCES `codes_new`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
INSERT INTO `code_usage_new`("code_id", "month", "usd") SELECT "code_id", "month", "usd" FROM `code_usage` WHERE "code_id" IN (SELECT "id" FROM `codes_new`);
--> statement-breakpoint
DROP TABLE `code_usage`;
--> statement-breakpoint
DROP TABLE `issuer_keys`;
--> statement-breakpoint
DROP TABLE `codes`;
--> statement-breakpoint
DROP TABLE `issuers`;
--> statement-breakpoint
ALTER TABLE `codes_new` RENAME TO `codes`;
--> statement-breakpoint
ALTER TABLE `code_usage_new` RENAME TO `code_usage`;
--> statement-breakpoint
CREATE UNIQUE INDEX `codes_codeHash_unique` ON `codes` (`code_hash`);
