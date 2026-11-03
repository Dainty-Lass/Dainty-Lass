CREATE TABLE `audit_events` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL,
	`actor_id` integer,
	`actor_name` text NOT NULL,
	`action` text NOT NULL,
	`target_type` text NOT NULL,
	`target_id` text,
	`target_label` text,
	`reason` text,
	`before` text,
	`after` text,
	`revertible` integer DEFAULT false NOT NULL,
	`revert_of_id` integer,
	`reverted_by_id` integer,
	`ip` text
);
--> statement-breakpoint
CREATE INDEX `audit_at` ON `audit_events` (`created_at`);--> statement-breakpoint
CREATE INDEX `audit_target` ON `audit_events` (`target_type`,`target_id`);--> statement-breakpoint
CREATE INDEX `audit_actor` ON `audit_events` (`actor_id`);--> statement-breakpoint
CREATE TABLE `blacklist_terms` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`pattern` text NOT NULL,
	`match_type` text NOT NULL,
	`action` text NOT NULL,
	`channels` text,
	`category` text DEFAULT 'general' NOT NULL,
	`note` text,
	`enabled` integer DEFAULT true NOT NULL,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL,
	`updated_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `blacklist_pattern` ON `blacklist_terms` (`pattern`,`match_type`);--> statement-breakpoint
CREATE TABLE `chat_messages` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`external_id` text,
	`at` integer NOT NULL,
	`channel` text NOT NULL,
	`room_id` text,
	`sender_user_id` text NOT NULL,
	`sender_name` text NOT NULL,
	`recipient_user_id` text,
	`recipient_name` text,
	`body` text NOT NULL,
	`verdict` text DEFAULT 'allowed' NOT NULL,
	`matched_term_ids` text
);
--> statement-breakpoint
CREATE INDEX `chat_at` ON `chat_messages` (`at`);--> statement-breakpoint
CREATE INDEX `chat_sender` ON `chat_messages` (`sender_user_id`,`at`);--> statement-breakpoint
CREATE INDEX `chat_recipient` ON `chat_messages` (`recipient_user_id`,`at`);--> statement-breakpoint
CREATE INDEX `chat_room` ON `chat_messages` (`room_id`,`at`);--> statement-breakpoint
CREATE UNIQUE INDEX `chat_external` ON `chat_messages` (`external_id`);--> statement-breakpoint
CREATE TABLE `config_revisions` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`kind` text NOT NULL,
	`version` integer NOT NULL,
	`snapshot` text NOT NULL,
	`summary` text NOT NULL,
	`actor_id` integer,
	`actor_name` text NOT NULL,
	`restored_from_id` integer,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `revisions_kind_version` ON `config_revisions` (`kind`,`version`);--> statement-breakpoint
CREATE TABLE `report_notes` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`report_id` integer NOT NULL,
	`staff_id` integer,
	`author_name` text NOT NULL,
	`body` text NOT NULL,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL,
	FOREIGN KEY (`report_id`) REFERENCES `reports`(`id`) ON UPDATE no action ON DELETE cascade,
	FOREIGN KEY (`staff_id`) REFERENCES `staff`(`id`) ON UPDATE no action ON DELETE set null
);
--> statement-breakpoint
CREATE INDEX `report_notes_report` ON `report_notes` (`report_id`);--> statement-breakpoint
CREATE TABLE `reports` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL,
	`updated_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL,
	`source` text NOT NULL,
	`external_id` text,
	`reporter_user_id` text,
	`reporter_name` text,
	`target_user_id` text,
	`target_name` text NOT NULL,
	`category` text NOT NULL,
	`description` text NOT NULL,
	`status` text DEFAULT 'open' NOT NULL,
	`priority` text DEFAULT 'normal' NOT NULL,
	`assignee_id` integer,
	`resolution` text,
	`resolved_at` integer,
	`context` text,
	FOREIGN KEY (`assignee_id`) REFERENCES `staff`(`id`) ON UPDATE no action ON DELETE set null
);
--> statement-breakpoint
CREATE INDEX `reports_status` ON `reports` (`status`,`created_at`);--> statement-breakpoint
CREATE INDEX `reports_target` ON `reports` (`target_user_id`);--> statement-breakpoint
CREATE UNIQUE INDEX `reports_external` ON `reports` (`source`,`external_id`);--> statement-breakpoint
CREATE TABLE `sessions` (
	`token_hash` text PRIMARY KEY NOT NULL,
	`staff_id` integer NOT NULL,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL,
	`expires_at` integer NOT NULL,
	`last_used_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL,
	`ip` text,
	`user_agent` text,
	FOREIGN KEY (`staff_id`) REFERENCES `staff`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE INDEX `sessions_staff` ON `sessions` (`staff_id`);--> statement-breakpoint
CREATE TABLE `settings` (
	`key` text PRIMARY KEY NOT NULL,
	`value` text NOT NULL,
	`updated_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL
);
--> statement-breakpoint
CREATE TABLE `staff` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`discord_id` text NOT NULL,
	`username` text NOT NULL,
	`global_name` text,
	`avatar` text,
	`role` text NOT NULL,
	`role_source` text DEFAULT 'manual' NOT NULL,
	`disabled` integer DEFAULT false NOT NULL,
	`note` text,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL,
	`last_seen_at` integer
);
--> statement-breakpoint
CREATE UNIQUE INDEX `staff_discord_id` ON `staff` (`discord_id`);