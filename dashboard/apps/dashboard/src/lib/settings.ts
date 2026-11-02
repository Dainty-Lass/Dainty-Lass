import { type Accent, DEFAULT_ACCENT, parseAccent } from "@dainty/theme/mode";
import { eq } from "drizzle-orm";
import type { Role } from "@/lib/auth/permissions";
import { db, schema } from "@/lib/db";

/**
 * Every setting a person tunes, with its default. One definition: the settings page, the revision
 * snapshot and the readers all go through it, so a new setting is an entry here and nowhere else.
 */
export interface Settings {
  /** Discord server whose roles set people's tier; empty means tiers are set here only. */
  discordGuildId: string;
  /** Discord role ids per dashboard role; the highest match wins. */
  discordRoleMap: Record<Exclude<Role, "owner">, string[]>;
  /** The Discord role that marks someone approved, and that accepting the terms adds. */
  discordApprovedRoleId: string;
  /** Send people who are not approved to the welcome page to create their account and accept the terms. */
  requireApproval: boolean;
  /** The server's terms, in Markdown, shown on the welcome page. */
  tosText: string;
  /** Bumped whenever the terms change, so those who accepted an older version accept again. Not set by hand. */
  tosVersion: number;
  /** How long chat lines are kept; 0 keeps them forever. */
  chatRetentionDays: number;
  /** Lines shown either side of a reported moment. */
  reportContextMinutes: number;
  /** Shown on the sign-in page and in the browser tab. */
  serverName: string;
  accent: Accent;
  /** Reasons offered when timing out or banning, so moderators pick rather than type. */
  moderationReasons: string[];
}

export const DEFAULT_SETTINGS: Settings = {
  discordGuildId: "",
  discordRoleMap: { admin: [], moderator: [], user: [] },
  discordApprovedRoleId: "",
  requireApproval: false,
  tosText: "",
  tosVersion: 1,
  chatRetentionDays: 90,
  reportContextMinutes: 10,
  serverName: "DaintyLass",
  accent: DEFAULT_ACCENT,
  moderationReasons: [
    "Harassment",
    "Hate speech",
    "Cheating",
    "Griefing",
    "Spam",
    "Inappropriate name",
  ],
};

export function getSettings(): Settings {
  const rows = db.select().from(schema.settings).all();
  const stored = Object.fromEntries(rows.map((r) => [r.key, r.value]));
  const merged = { ...DEFAULT_SETTINGS, ...(stored as Partial<Settings>) };
  // Saved before "viewer" became "user".
  const map = merged.discordRoleMap as Settings["discordRoleMap"] & { viewer?: string[] };
  if (map.viewer && !map.user)
    merged.discordRoleMap = { admin: map.admin, moderator: map.moderator, user: map.viewer };
  // An accent saved before the palette changed falls back to the default rather than to nothing.
  return { ...merged, accent: parseAccent(merged.accent) };
}

export function getSetting<K extends keyof Settings>(key: K): Settings[K] {
  const row = db.select().from(schema.settings).where(eq(schema.settings.key, key)).get();
  return (row?.value as Settings[K] | undefined) ?? DEFAULT_SETTINGS[key];
}

/** Writes inside the caller's transaction when given one, so a revision and its audit land together. */
export function writeSettings(patch: Partial<Settings>, t: Pick<typeof db, "insert"> = db): void {
  for (const [key, value] of Object.entries(patch)) {
    t.insert(schema.settings)
      .values({ key, value, updatedAt: Date.now() })
      .onConflictDoUpdate({ target: schema.settings.key, set: { value, updatedAt: Date.now() } })
      .run();
  }
}
