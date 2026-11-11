-- 管理画面（DESIGN.md 17.3）。ベータで使う開発者のキー（複数）と、ベータの設定
CREATE TABLE `beta_keys` (
	`id` text PRIMARY KEY NOT NULL,
	`provider` text NOT NULL,
	`encrypted_key` text NOT NULL,
	`tail` text NOT NULL,
	`memo` text DEFAULT '' NOT NULL,
	`enabled` integer DEFAULT true NOT NULL,
	`failures` integer DEFAULT 0 NOT NULL,
	`failed_at` integer,
	`failed_status` integer,
	`created_at` integer NOT NULL
);
--> statement-breakpoint
CREATE INDEX `beta_keys_provider` ON `beta_keys` (`provider`);--> statement-breakpoint
CREATE TABLE `beta_settings` (
	`name` text PRIMARY KEY NOT NULL,
	`value` text NOT NULL
);
