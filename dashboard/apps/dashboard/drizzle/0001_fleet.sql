CREATE TABLE `controller_samples` (
	`controller_id` text NOT NULL,
	`at` integer NOT NULL,
	`online` integer NOT NULL,
	`hunts_running` integer NOT NULL,
	`hunts_max` integer NOT NULL,
	PRIMARY KEY(`controller_id`, `at`)
);
--> statement-breakpoint
CREATE TABLE `controllers` (
	`id` text PRIMARY KEY NOT NULL,
	`kind` text NOT NULL,
	`name` text NOT NULL,
	`region` text,
	`note` text,
	`token_hash` text,
	`pairing_code_hash` text,
	`pairing_expires_at` integer,
	`paired_at` integer,
	`last_seen_at` integer,
	`version` text,
	`status` text,
	`disabled` integer DEFAULT false NOT NULL,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL
);
--> statement-breakpoint
DROP INDEX `chat_external`;--> statement-breakpoint
ALTER TABLE `chat_messages` ADD `controller_id` text DEFAULT 'local' NOT NULL;--> statement-breakpoint
CREATE INDEX `chat_controller` ON `chat_messages` (`controller_id`,`at`);--> statement-breakpoint
CREATE UNIQUE INDEX `chat_external` ON `chat_messages` (`controller_id`,`external_id`);--> statement-breakpoint
DROP INDEX `reports_external`;--> statement-breakpoint
ALTER TABLE `reports` ADD `controller_id` text DEFAULT 'local' NOT NULL;--> statement-breakpoint
CREATE INDEX `reports_controller` ON `reports` (`controller_id`,`status`);--> statement-breakpoint
CREATE UNIQUE INDEX `reports_external` ON `reports` (`controller_id`,`source`,`external_id`);--> statement-breakpoint
ALTER TABLE `audit_events` ADD `controller_id` text DEFAULT 'local' NOT NULL;--> statement-breakpoint
CREATE INDEX `audit_controller` ON `audit_events` (`controller_id`,`created_at`);