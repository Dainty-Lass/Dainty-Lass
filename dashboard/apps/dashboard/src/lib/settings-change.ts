import { type Actor, recordAudit } from "@/lib/audit/record";
import { tx } from "@/lib/db";
import { DomainError } from "@/lib/errors";
import { ensureBaseline, writeRevision } from "@/lib/revisions";
import { getSettings, type Settings, writeSettings } from "@/lib/settings";

const ACCENTS = ["periwinkle", "gold", "green", "pink", "purple"];
const ids = (list: unknown) =>
  Array.isArray(list)
    ? [
        ...new Set(
          list
            .map(String)
            .map((s) => s.trim())
            .filter((s) => /^\d{17,20}$/.test(s)),
        ),
      ]
    : [];

/** Validates a patch against the definitions in settings.ts; anything unknown is dropped. */
export function sanitize(patch: Partial<Settings>): Partial<Settings> {
  const out: Partial<Settings> = {};
  if (patch.discordGuildId !== undefined) {
    const g = String(patch.discordGuildId).trim();
    if (g && !/^\d{17,20}$/.test(g)) throw new DomainError("invalid_input", "discordGuildId");
    out.discordGuildId = g;
  }
  if (patch.discordRoleMap !== undefined) {
    out.discordRoleMap = {
      admin: ids(patch.discordRoleMap.admin),
      moderator: ids(patch.discordRoleMap.moderator),
      viewer: ids(patch.discordRoleMap.viewer),
    };
  }
  if (patch.chatRetentionDays !== undefined) {
    const d = Math.trunc(Number(patch.chatRetentionDays));
    if (!(d >= 0 && d <= 3650)) throw new DomainError("invalid_input", "chatRetentionDays");
    out.chatRetentionDays = d;
  }
  if (patch.reportContextMinutes !== undefined) {
    const m = Math.trunc(Number(patch.reportContextMinutes));
    if (!(m >= 1 && m <= 120)) throw new DomainError("invalid_input", "reportContextMinutes");
    out.reportContextMinutes = m;
  }
  if (patch.serverName !== undefined) {
    const n = String(patch.serverName).trim();
    if (!n || n.length > 64) throw new DomainError("invalid_input", "serverName");
    out.serverName = n;
  }
  if (patch.accent !== undefined) {
    if (!ACCENTS.includes(patch.accent)) throw new DomainError("invalid_input", "accent");
    out.accent = patch.accent;
  }
  if (patch.moderationReasons !== undefined) {
    out.moderationReasons = [
      ...new Set(
        patch.moderationReasons
          .map((r) => String(r).trim())
          .filter((r) => r.length >= 3 && r.length <= 80),
      ),
    ].slice(0, 20);
  }
  return out;
}

export function updateSettings(actor: Actor, patch: Partial<Settings>): Settings {
  const clean = sanitize(patch);
  return tx((t) => {
    const replaced = ensureBaseline("settings", t);
    const before = getSettings();
    writeSettings(clean, t);
    const after = { ...before, ...clean };
    const changed = (Object.keys(clean) as (keyof Settings)[]).filter(
      (k) => JSON.stringify(before[k]) !== JSON.stringify(after[k]),
    );
    if (changed.length === 0) return after;
    const revision = writeRevision("settings", `edit:${changed.join(",")}`, actor, t);
    recordAudit(
      actor,
      {
        action: "settings.update",
        targetType: "settings",
        targetId: "settings",
        targetLabel: changed.join(", "),
        // The revision ids are what a revert restores; the values are what the log shows.
        before: {
          revisionId: replaced.id,
          ...Object.fromEntries(changed.map((k) => [k, before[k]])),
        },
        after: {
          revisionId: revision.id,
          ...Object.fromEntries(changed.map((k) => [k, after[k]])),
        },
        revertible: true,
      },
      t,
    );
    return after;
  });
}
