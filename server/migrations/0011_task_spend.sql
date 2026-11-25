CREATE TABLE `task_spend` (
	`day` text NOT NULL,
	`task` text NOT NULL,
	`model` text NOT NULL,
	`usd` real DEFAULT 0 NOT NULL,
	`count` integer DEFAULT 0 NOT NULL,
	PRIMARY KEY(`day`, `task`, `model`)
);
