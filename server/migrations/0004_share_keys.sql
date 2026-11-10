-- 預けるキーを会社ごとの表（issuer_keys）に分け、OpenAI のキーがなくてもおすそ分けできるようにする（DESIGN.md 17.1）。
-- 合言葉は issuers の id と token_salt から決める。今の合言葉（admin_token_hash）はそのまま使える。
-- 預けた xAI のキーはハッシュを持っていなかったので移さない（預け直す）。
-- D1 は外部キーを常に確かめるので、表を消すと ON DELETE CASCADE で子の行も消える。
-- そこで新しい表を別の名前で作って写し、古い表を子から順に消してから名前を戻す（名前を戻すと外部キーの参照先も直る）。
-- 形は drizzle-kit が schema.ts から作るものと同じ（meta/0004_snapshot.json）
CREATE TABLE `new_issuers` (
	`id` text PRIMARY KEY NOT NULL,
	`admin_token_hash` text NOT NULL,
	`token_salt` text NOT NULL,
	`created_at` integer NOT NULL
);
CREATE TABLE `new_issuer_keys` (
	`issuer_id` text NOT NULL,
	`provider` text NOT NULL,
	`key_hash` text NOT NULL,
	`encrypted_key` text NOT NULL,
	PRIMARY KEY(`issuer_id`, `provider`),
	FOREIGN KEY (`issuer_id`) REFERENCES `new_issuers`(`id`) ON UPDATE no action ON DELETE cascade
);
CREATE TABLE `new_codes` (
	`id` text PRIMARY KEY NOT NULL,
	`code_hash` text NOT NULL,
	`issuer_id` text NOT NULL,
	`memo` text DEFAULT '' NOT NULL,
	`expires_at` integer,
	`revoked` integer DEFAULT false NOT NULL,
	`created_at` integer NOT NULL,
	`daily_limit_usd` real,
	`day` text,
	`day_usd` real DEFAULT 0 NOT NULL,
	FOREIGN KEY (`issuer_id`) REFERENCES `new_issuers`(`id`) ON UPDATE no action ON DELETE cascade
);
CREATE TABLE `new_code_usage` (
	`code_id` text NOT NULL,
	`month` text NOT NULL,
	`usd` real DEFAULT 0 NOT NULL,
	PRIMARY KEY(`code_id`, `month`),
	FOREIGN KEY (`code_id`) REFERENCES `new_codes`(`id`) ON UPDATE no action ON DELETE cascade
);
INSERT INTO `new_issuers` SELECT `id`, `admin_token_hash`, lower(hex(randomblob(16))), `created_at` FROM `issuers`;
INSERT INTO `new_issuer_keys` SELECT `id`, 'openai', `key_hash`, `encrypted_key` FROM `issuers`;
INSERT INTO `new_codes` SELECT `id`, `code_hash`, `issuer_id`, `memo`, `expires_at`, `revoked`, `created_at`, `daily_limit_usd`, `day`, `day_usd` FROM `codes`;
INSERT INTO `new_code_usage` SELECT `code_id`, `month`, `usd` FROM `code_usage`;
DROP TABLE `code_usage`;
DROP TABLE `codes`;
DROP TABLE `issuers`;
ALTER TABLE `new_issuers` RENAME TO `issuers`;
ALTER TABLE `new_issuer_keys` RENAME TO `issuer_keys`;
ALTER TABLE `new_codes` RENAME TO `codes`;
ALTER TABLE `new_code_usage` RENAME TO `code_usage`;
CREATE UNIQUE INDEX `issuers_adminTokenHash_unique` ON `issuers` (`admin_token_hash`);
CREATE UNIQUE INDEX `issuer_keys_keyHash_unique` ON `issuer_keys` (`key_hash`);
CREATE UNIQUE INDEX `codes_codeHash_unique` ON `codes` (`code_hash`);
CREATE INDEX `codes_issuer` ON `codes` (`issuer_id`);
