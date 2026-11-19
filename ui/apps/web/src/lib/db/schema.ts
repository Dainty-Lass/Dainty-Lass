import { sql } from "drizzle-orm";
import {
  index,
  integer,
  primaryKey,
  sqliteTable,
  text,
  uniqueIndex,
} from "drizzle-orm/sqlite-core";

const now = sql`(unixepoch('subsec') * 1000)`;
const createdAt = () => integer("created_at", { mode: "number" }).notNull().default(now);
// The controller a row came from or acted on; "local" is the one this dashboard runs on.
const controllerId = () => text("controller_id").notNull().default("local");

// Better Auth's tables (lib/auth/server.ts). Keys are its field names; times are Date in milliseconds.
const authTime = (name: string) => integer(name, { mode: "timestamp_ms" });

/** A sign-in identity: one per person, whichever of Discord, Twitch, an email code or a passkey they use. */
export const authUsers = sqliteTable("auth_users", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull(),
  email: text("email").notNull().unique(),
  emailVerified: integer("email_verified", { mode: "boolean" }).notNull().default(false),
  image: text("image"),
  createdAt: authTime("created_at").notNull(),
  updatedAt: authTime("updated_at").notNull(),
});

export const authSessions = sqliteTable(
  "auth_sessions",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    userId: integer("user_id")
      .notNull()
      .references(() => authUsers.id, { onDelete: "cascade" }),
    token: text("token").notNull().unique(),
    expiresAt: authTime("expires_at").notNull(),
    ipAddress: text("ip_address"),
    userAgent: text("user_agent"),
    createdAt: authTime("created_at").notNull(),
    updatedAt: authTime("updated_at").notNull(),
  },
  (t) => [index("auth_sessions_user").on(t.userId)],
);

/** A linked provider: discord, twitch. Tokens are encrypted with the auth secret. No passwords, ever. */
export const authAccounts = sqliteTable(
  "auth_accounts",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    userId: integer("user_id")
      .notNull()
      .references(() => authUsers.id, { onDelete: "cascade" }),
    accountId: text("account_id").notNull(),
    providerId: text("provider_id").notNull(),
    accessToken: text("access_token"),
    refreshToken: text("refresh_token"),
    idToken: text("id_token"),
    accessTokenExpiresAt: authTime("access_token_expires_at"),
    refreshTokenExpiresAt: authTime("refresh_token_expires_at"),
    scope: text("scope"),
    password: text("password"),
    createdAt: authTime("created_at").notNull(),
    updatedAt: authTime("updated_at").notNull(),
  },
  (t) => [
    uniqueIndex("auth_accounts_provider").on(t.providerId, t.accountId),
    index("auth_accounts_user").on(t.userId),
  ],
);

/** Email codes and OAuth state. */
export const authVerifications = sqliteTable("auth_verifications", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  identifier: text("identifier").notNull(),
  value: text("value").notNull(),
  expiresAt: authTime("expires_at").notNull(),
  createdAt: authTime("created_at"),
  updatedAt: authTime("updated_at"),
});

export const authPasskeys = sqliteTable(
  "auth_passkeys",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    name: text("name"),
    publicKey: text("public_key").notNull(),
    userId: integer("user_id")
      .notNull()
      .references(() => authUsers.id, { onDelete: "cascade" }),
    credentialID: text("credential_id").notNull().unique(),
    counter: integer("counter").notNull(),
    deviceType: text("device_type").notNull(),
    backedUp: integer("backed_up", { mode: "boolean" }).notNull(),
    transports: text("transports"),
    aaguid: text("aaguid"),
    createdAt: authTime("created_at"),
  },
  (t) => [index("auth_passkeys_user").on(t.userId)],
);

/**
 * What the dashboard knows of each person who has signed in: one row per sign-in identity. Anyone may sign
 * in; `role` decides what they see, and `approvedAt` whether they are past onboarding when the server asks.
 * `discordId` is set once a Discord account is linked: guild roles and the game's operator id need it.
 */
export const staff = sqliteTable(
  "staff",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    userId: integer("user_id")
      .notNull()
      .references(() => authUsers.id, { onDelete: "cascade" }),
    discordId: text("discord_id"),
    // The name shown until a display name is chosen: the provider's, or the email's local part.
    username: text("username").notNull(),
    globalName: text("global_name"),
    // A full URL from whichever provider signed in last.
    avatar: text("avatar"),
    role: text("role", { enum: ["owner", "admin", "moderator", "user"] }).notNull(),
    // manual: set here and kept. guild: from a Discord role, re-read at every sign-in. default: no role
    // granted anything, so the account is a plain user until one does.
    roleSource: text("role_source", { enum: ["manual", "guild", "default"] })
      .notNull()
      .default("manual"),
    disabled: integer("disabled", { mode: "boolean" }).notNull().default(false),
    note: text("note"),
    // Chosen on the welcome page; the provider's name is shown until then.
    displayName: text("display_name"),
    // The in-game name the person says is theirs, so staff can find them among players.
    gameName: text("game_name"),
    approvedAt: integer("approved_at", { mode: "number" }),
    approvedVia: text("approved_via", { enum: ["tos", "guild", "manual"] }),
    // The terms version accepted; a newer version asks again of those approved by accepting.
    tosVersion: integer("tos_version"),
    createdAt: createdAt(),
    lastSeenAt: integer("last_seen_at", { mode: "number" }),
  },
  (t) => [uniqueIndex("staff_user").on(t.userId), uniqueIndex("staff_discord_id").on(t.discordId)],
);

/**
 * Every change anyone makes, here or through the game's admin API on this dashboard's behalf. `before`
 * and `after` are what a revert needs; `revertedById` points at the event that undid this one.
 */
export const auditEvents = sqliteTable(
  "audit_events",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    at: createdAt(),
    actorId: integer("actor_id"),
    actorName: text("actor_name").notNull(),
    controllerId: controllerId(),
    action: text("action").notNull(),
    targetType: text("target_type").notNull(),
    targetId: text("target_id"),
    targetLabel: text("target_label"),
    reason: text("reason"),
    before: text("before", { mode: "json" }).$type<unknown>(),
    after: text("after", { mode: "json" }).$type<unknown>(),
    revertible: integer("revertible", { mode: "boolean" }).notNull().default(false),
    revertOfId: integer("revert_of_id"),
    revertedById: integer("reverted_by_id"),
    ip: text("ip"),
  },
  (t) => [
    index("audit_at").on(t.at),
    index("audit_target").on(t.targetType, t.targetId),
    index("audit_actor").on(t.actorId),
    index("audit_controller").on(t.controllerId, t.at),
  ],
);

/** Player reports, from the game (ingest), from Discord or entered by staff. */
export const reports = sqliteTable(
  "reports",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    createdAt: createdAt(),
    updatedAt: integer("updated_at", { mode: "number" }).notNull().default(now),
    controllerId: controllerId(),
    source: text("source", { enum: ["game", "discord", "dashboard"] }).notNull(),
    // The sender's own id for this report, so a retried ingest does not file it twice.
    externalId: text("external_id"),
    reporterUserId: text("reporter_user_id"),
    reporterName: text("reporter_name"),
    targetUserId: text("target_user_id"),
    targetName: text("target_name").notNull(),
    category: text("category", {
      enum: [
        "harassment",
        "hate_speech",
        "cheating",
        "griefing",
        "spam",
        "inappropriate_name",
        "real_money_trading",
        "other",
      ],
    }).notNull(),
    description: text("description").notNull(),
    status: text("status", { enum: ["open", "claimed", "resolved", "dismissed"] })
      .notNull()
      .default("open"),
    priority: text("priority", { enum: ["low", "normal", "high"] })
      .notNull()
      .default("normal"),
    assigneeId: integer("assignee_id").references(() => staff.id, { onDelete: "set null" }),
    resolution: text("resolution"),
    resolvedAt: integer("resolved_at", { mode: "number" }),
    // Where in the game it happened: a room or instance id, for pulling the chat around it.
    context: text("context", { mode: "json" }).$type<{
      roomId?: string;
      instanceId?: string;
      at?: number;
    }>(),
  },
  (t) => [
    index("reports_status").on(t.status, t.createdAt),
    index("reports_target").on(t.targetUserId),
    uniqueIndex("reports_external").on(t.controllerId, t.source, t.externalId),
    index("reports_controller").on(t.controllerId, t.status),
  ],
);

export const reportNotes = sqliteTable(
  "report_notes",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    reportId: integer("report_id")
      .notNull()
      .references(() => reports.id, { onDelete: "cascade" }),
    staffId: integer("staff_id").references(() => staff.id, { onDelete: "set null" }),
    authorName: text("author_name").notNull(),
    body: text("body").notNull(),
    at: createdAt(),
  },
  (t) => [index("report_notes_report").on(t.reportId)],
);

/** Chat as the backend relayed it: city rooms, parties, guilds and direct messages. */
export const chatMessages = sqliteTable(
  "chat_messages",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    // The backend's id for the stanza, so a retried batch is not stored twice.
    externalId: text("external_id"),
    controllerId: controllerId(),
    at: integer("at", { mode: "number" }).notNull(),
    channel: text("channel", { enum: ["city", "party", "guild", "direct", "system"] }).notNull(),
    roomId: text("room_id"),
    senderUserId: text("sender_user_id").notNull(),
    senderName: text("sender_name").notNull(),
    recipientUserId: text("recipient_user_id"),
    recipientName: text("recipient_name"),
    body: text("body").notNull(),
    // What the filter did, as the backend enforced it (or as the dashboard rated it on arrival).
    verdict: text("verdict", { enum: ["allowed", "flagged", "masked", "blocked"] })
      .notNull()
      .default("allowed"),
    matchedTermIds: text("matched_term_ids", { mode: "json" }).$type<number[]>(),
  },
  (t) => [
    index("chat_at").on(t.at),
    index("chat_sender").on(t.senderUserId, t.at),
    index("chat_recipient").on(t.recipientUserId, t.at),
    index("chat_room").on(t.roomId, t.at),
    uniqueIndex("chat_external").on(t.controllerId, t.externalId),
    index("chat_controller").on(t.controllerId, t.at),
  ],
);

/** The chat filter's terms. Every change also writes a config revision, which is what rollback restores. */
export const blacklistTerms = sqliteTable(
  "blacklist_terms",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    pattern: text("pattern").notNull(),
    matchType: text("match_type", { enum: ["word", "substring", "regex"] }).notNull(),
    action: text("action", { enum: ["flag", "mask", "block"] }).notNull(),
    channels: text("channels", { mode: "json" }).$type<string[] | null>(),
    category: text("category").notNull().default("general"),
    note: text("note"),
    enabled: integer("enabled", { mode: "boolean" }).notNull().default(true),
    createdAt: createdAt(),
    updatedAt: integer("updated_at", { mode: "number" }).notNull().default(now),
  },
  (t) => [uniqueIndex("blacklist_pattern").on(t.pattern, t.matchType)],
);

/**
 * Whole-document snapshots of what can be rolled back as one: the chat filter, the settings and the
 * staff list. Restoring writes a new revision rather than deleting the newer ones.
 */
export const configRevisions = sqliteTable(
  "config_revisions",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    kind: text("kind", { enum: ["chat_filter", "settings", "staff"] }).notNull(),
    version: integer("version").notNull(),
    snapshot: text("snapshot", { mode: "json" }).$type<unknown>().notNull(),
    summary: text("summary").notNull(),
    actorId: integer("actor_id"),
    actorName: text("actor_name").notNull(),
    restoredFromId: integer("restored_from_id"),
    at: createdAt(),
  },
  (t) => [uniqueIndex("revisions_kind_version").on(t.kind, t.version)],
);

export const settings = sqliteTable("settings", {
  key: text("key").primaryKey(),
  value: text("value", { mode: "json" }).$type<unknown>().notNull(),
  updatedAt: integer("updated_at", { mode: "number" }).notNull().default(now),
});

/**
 * The game servers' hosts. "local" is the one the dashboard runs on and calls directly; every other one
 * runs the agent (apps/agent), which dials in. A token or pairing code is kept only as its SHA-256.
 */
export const controllers = sqliteTable("controllers", {
  id: text("id").primaryKey(),
  kind: text("kind", { enum: ["local", "agent"] }).notNull(),
  name: text("name").notNull(),
  region: text("region"),
  note: text("note"),
  tokenHash: text("token_hash"),
  pairingCodeHash: text("pairing_code_hash"),
  pairingExpiresAt: integer("pairing_expires_at", { mode: "number" }),
  pairedAt: integer("paired_at", { mode: "number" }),
  lastSeenAt: integer("last_seen_at", { mode: "number" }),
  version: text("version"),
  // The last AgentStatus (@dainty/fleet), so pages read status without asking any controller.
  status: text("status", { mode: "json" }).$type<unknown>(),
  disabled: integer("disabled", { mode: "boolean" }).notNull().default(false),
  createdAt: createdAt(),
});

/** A point a minute per controller, for the fleet's history charts; pruned after a week. */
export const controllerSamples = sqliteTable(
  "controller_samples",
  {
    controllerId: text("controller_id").notNull(),
    at: integer("at", { mode: "number" }).notNull(),
    online: integer("online").notNull(),
    huntsRunning: integer("hunts_running").notNull(),
    huntsMax: integer("hunts_max").notNull(),
  },
  (t) => [primaryKey({ columns: [t.controllerId, t.at] })],
);

/** Player crash reports the content service relays; the full files stay on the game server at `server_path`. */
export const crashReports = sqliteTable(
  "crash_reports",
  {
    // The sender's 32-hex id, so a retried ingest is a duplicate rather than a second row.
    id: text("id").primaryKey(),
    controllerId: controllerId(),
    receivedAt: integer("received_at", { mode: "number" }).notNull(),
    userId: text("user_id").notNull(),
    userName: text("user_name").notNull(),
    crashId: text("crash_id").notNull(),
    gameVersion: text("game_version"),
    launcherVersion: text("launcher_version"),
    exitCode: text("exit_code"),
    serverPath: text("server_path").notNull(),
    files: text("files", { mode: "json" })
      .notNull()
      .$type<{ name: string; size: number; sha256: string; text: string | null }[]>(),
    createdAt: createdAt(),
  },
  (t) => [
    index("crash_reports_received").on(t.receivedAt),
    index("crash_reports_controller").on(t.controllerId, t.receivedAt),
  ],
);

export type Staff = typeof staff.$inferSelect;
export type AuditEvent = typeof auditEvents.$inferSelect;
export type Report = typeof reports.$inferSelect;
export type ReportNote = typeof reportNotes.$inferSelect;
export type ChatMessage = typeof chatMessages.$inferSelect;
export type BlacklistTerm = typeof blacklistTerms.$inferSelect;
export type ConfigRevision = typeof configRevisions.$inferSelect;
export type Controller = typeof controllers.$inferSelect;
export type CrashReport = typeof crashReports.$inferSelect;
