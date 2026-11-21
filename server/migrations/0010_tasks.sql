CREATE TABLE `tasks` (
	`name` text PRIMARY KEY NOT NULL,
	`model` text NOT NULL,
	`enabled` integer DEFAULT true NOT NULL
);
