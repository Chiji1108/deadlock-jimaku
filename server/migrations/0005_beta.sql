-- ベータのコードを出した数（DESIGN.md 17.2）。同じ所（IP アドレスと日付のハッシュ）に出しすぎないために数える
CREATE TABLE `beta_grants` (
	`place` text NOT NULL,
	`day` text NOT NULL,
	`count` integer DEFAULT 0 NOT NULL,
	PRIMARY KEY(`place`, `day`)
);
