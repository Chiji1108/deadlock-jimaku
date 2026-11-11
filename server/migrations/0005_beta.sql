-- ベータの間の無料の提供（DESIGN.md 17.2）。コードを出した数（同じ所に出しすぎない）と、その日に使った額（全体の上限）
CREATE TABLE `beta_grants` (
	`day` text NOT NULL,
	`place` text NOT NULL,
	`count` integer DEFAULT 0 NOT NULL,
	PRIMARY KEY(`day`, `place`)
);
--> statement-breakpoint
CREATE TABLE `beta_spend` (
	`day` text PRIMARY KEY NOT NULL,
	`usd` real DEFAULT 0 NOT NULL
);
