CREATE TABLE `auth_accounts` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`user_id` integer NOT NULL,
	`account_id` text NOT NULL,
	`provider_id` text NOT NULL,
	`access_token` text,
	`refresh_token` text,
	`id_token` text,
	`access_token_expires_at` integer,
	`refresh_token_expires_at` integer,
	`scope` text,
	`password` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`user_id`) REFERENCES `auth_users`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE UNIQUE INDEX `auth_accounts_provider` ON `auth_accounts` (`provider_id`,`account_id`);--> statement-breakpoint
CREATE INDEX `auth_accounts_user` ON `auth_accounts` (`user_id`);--> statement-breakpoint
CREATE TABLE `auth_passkeys` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`name` text,
	`public_key` text NOT NULL,
	`user_id` integer NOT NULL,
	`credential_id` text NOT NULL,
	`counter` integer NOT NULL,
	`device_type` text NOT NULL,
	`backed_up` integer NOT NULL,
	`transports` text,
	`aaguid` text,
	`created_at` integer,
	FOREIGN KEY (`user_id`) REFERENCES `auth_users`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE UNIQUE INDEX `auth_passkeys_credential_id_unique` ON `auth_passkeys` (`credential_id`);--> statement-breakpoint
CREATE INDEX `auth_passkeys_user` ON `auth_passkeys` (`user_id`);--> statement-breakpoint
CREATE TABLE `auth_sessions` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`user_id` integer NOT NULL,
	`token` text NOT NULL,
	`expires_at` integer NOT NULL,
	`ip_address` text,
	`user_agent` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`user_id`) REFERENCES `auth_users`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE UNIQUE INDEX `auth_sessions_token_unique` ON `auth_sessions` (`token`);--> statement-breakpoint
CREATE INDEX `auth_sessions_user` ON `auth_sessions` (`user_id`);--> statement-breakpoint
CREATE TABLE `auth_users` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`name` text NOT NULL,
	`email` text NOT NULL,
	`email_verified` integer DEFAULT false NOT NULL,
	`image` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `auth_users_email_unique` ON `auth_users` (`email`);--> statement-breakpoint
CREATE TABLE `auth_verifications` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`identifier` text NOT NULL,
	`value` text NOT NULL,
	`expires_at` integer NOT NULL,
	`created_at` integer,
	`updated_at` integer
);
--> statement-breakpoint
-- Every existing member was a Discord sign-in: give each a sign-in identity with the same id, and link their
-- Discord account so the next Discord sign-in finds them. The address is a placeholder until a real one is linked.
INSERT INTO `auth_users`("id", "name", "email", "email_verified", "image", "created_at", "updated_at")
SELECT "id", coalesce("global_name", "username"), 'discord-' || "discord_id" || '@users.invalid', 0,
  CASE WHEN "avatar" IS NULL THEN NULL ELSE 'https://cdn.discordapp.com/avatars/' || "discord_id" || '/' || "avatar" || '.webp?size=128' END,
  "created_at", "created_at" FROM `staff`;--> statement-breakpoint
INSERT INTO `auth_accounts`("user_id", "account_id", "provider_id", "created_at", "updated_at")
SELECT "id", "discord_id", 'discord', "created_at", "created_at" FROM `staff`;--> statement-breakpoint
PRAGMA foreign_keys=OFF;--> statement-breakpoint
CREATE TABLE `__new_staff` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`user_id` integer NOT NULL,
	`discord_id` text,
	`username` text NOT NULL,
	`global_name` text,
	`avatar` text,
	`role` text NOT NULL,
	`role_source` text DEFAULT 'manual' NOT NULL,
	`disabled` integer DEFAULT false NOT NULL,
	`note` text,
	`display_name` text,
	`game_name` text,
	`approved_at` integer,
	`approved_via` text,
	`tos_version` integer,
	`created_at` integer DEFAULT (unixepoch('subsec') * 1000) NOT NULL,
	`last_seen_at` integer,
	FOREIGN KEY (`user_id`) REFERENCES `auth_users`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
INSERT INTO `__new_staff`("id", "user_id", "discord_id", "username", "global_name", "avatar", "role", "role_source", "disabled", "note", "created_at", "last_seen_at")
SELECT "id", "id", "discord_id", "username", "global_name",
  CASE WHEN "avatar" IS NULL THEN NULL ELSE 'https://cdn.discordapp.com/avatars/' || "discord_id" || '/' || "avatar" || '.webp?size=128' END,
  CASE "role" WHEN 'viewer' THEN 'user' ELSE "role" END, "role_source", "disabled", "note", "created_at", "last_seen_at" FROM `staff`;--> statement-breakpoint
DROP TABLE `staff`;--> statement-breakpoint
ALTER TABLE `__new_staff` RENAME TO `staff`;--> statement-breakpoint
PRAGMA foreign_keys=ON;--> statement-breakpoint
CREATE UNIQUE INDEX `staff_user` ON `staff` (`user_id`);--> statement-breakpoint
CREATE UNIQUE INDEX `staff_discord_id` ON `staff` (`discord_id`);