import { eq } from "drizzle-orm";
import type { Role } from "@/lib/auth/permissions";
import { db, schema } from "@/lib/db";

/**
 * Every setting a person tunes, with its default. One definition: the settings page, the revision
 * snapshot and the readers all go through it, so a new setting is an entry here and nowhere else.
 */
export interface Settings {
  /** Discord server whose roles grant access; empty turns guild sign-in off. */
  discordGuildId: string;
  /** Discord role ids per dashboard role; the highest match wins. */
  discordRoleMap: Record<Exclude<Role, "owner">, string[]>;
  /** How long chat lines are kept; 0 keeps them forever. */
  chatRetentionDays: number;
  /** Lines shown either side of a reported moment. */
  reportContextMinutes: number;
  /** Shown on the sign-in page and in the browser tab. */
  serverName: string;
  accent: "periwinkle" | "gold" | "green" | "pink" | "purple";
  /** Reasons offered when timing out or banning, so moderators pick rather than type. */
  moderationReasons: string[];
}

export const DEFAULT_SETTINGS: Settings = {
  discordGuildId: "",
  discordRoleMap: { admin: [], moderator: [], viewer: [] },
  chatRetentionDays: 90,
  reportContextMinutes: 10,
  serverName: "DaintyLass",
  accent: "periwinkle",
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
  return { ...DEFAULT_SETTINGS, ...(stored as Partial<Settings>) };
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
