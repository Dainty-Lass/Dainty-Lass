CREATE TABLE `crash_reports` (
	`id` text PRIMARY KEY NOT NULL,
	`controller_id` text DEFAULT 'local' NOT NULL,
	`received_at` integer NOT NULL,
	`user_id` text NOT NULL,
	`user_name` text NOT NULL,
	`crash_id` text NOT NULL,
	`game_version` text,
	`launcher_version` text,
	`exit_code` text,
	`server_path` text NOT NULL,
	`files` text NOT NULL,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL
);
--> statement-breakpoint
CREATE INDEX `crash_reports_received` ON `crash_reports` (`received_at`);--> statement-breakpoint
CREATE INDEX `crash_reports_controller` ON `crash_reports` (`controller_id`,`received_at`);