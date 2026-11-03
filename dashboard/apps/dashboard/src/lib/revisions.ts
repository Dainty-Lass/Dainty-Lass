import { desc, eq, max } from "drizzle-orm";
import { type Actor, actorFields, recordAudit } from "@/lib/audit/record";
import { db, schema, tx } from "@/lib/db";
import type { BlacklistTerm, ConfigRevision } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { DEFAULT_SETTINGS, getSettings, type Settings, writeSettings } from "@/lib/settings";

/**
 * Whole-document history for what is rolled back as one. A change writes the new state as the next
 * version; restoring an old version writes it again as the newest, so history only grows.
 */
export type RevisionKind = ConfigRevision["kind"];
type Tx = Parameters<Parameters<typeof db.transaction>[0]>[0];

export type FilterSnapshot = Omit<BlacklistTerm, "createdAt" | "updatedAt">[];
export type StaffSnapshot = {
  // Older snapshots name people by Discord id only.
  id?: number;
  discordId: string | null;
  username: string;
  role: string;
  disabled: boolean;
  roleSource: string;
}[];

export function snapshotFilter(t: Pick<Tx, "select"> = db): FilterSnapshot {
  return t
    .select({
      id: schema.blacklistTerms.id,
      pattern: schema.blacklistTerms.pattern,
      matchType: schema.blacklistTerms.matchType,
      action: schema.blacklistTerms.action,
      channels: schema.blacklistTerms.channels,
      category: schema.blacklistTerms.category,
      note: schema.blacklistTerms.note,
      enabled: schema.blacklistTerms.enabled,
    })
    .from(schema.blacklistTerms)
    .orderBy(schema.blacklistTerms.id)
    .all();
}

function snapshotStaff(t: Pick<Tx, "select">): StaffSnapshot {
  return t
    .select({
      id: schema.staff.id,
      discordId: schema.staff.discordId,
      username: schema.staff.username,
      role: schema.staff.role,
      disabled: schema.staff.disabled,
      roleSource: schema.staff.roleSource,
    })
    .from(schema.staff)
    .orderBy(schema.staff.id)
    .all();
}

export function snapshotOf(kind: RevisionKind, t: Tx | typeof db = db): unknown {
  if (kind === "chat_filter") return snapshotFilter(t);
  if (kind === "staff") return snapshotStaff(t);
  return getSettings();
}

function latestRevision(kind: RevisionKind, t: Pick<Tx, "select">): ConfigRevision | undefined {
  return t
    .select()
    .from(schema.configRevisions)
    .where(eq(schema.configRevisions.kind, kind))
    .orderBy(desc(schema.configRevisions.version))
    .limit(1)
    .get();
}

/**
 * Call inside the change's transaction, before changing anything: the first change of a kind first
 * records the state it started from, so even that change can be rolled back. Returns the revision the
 * change replaces.
 */
export function ensureBaseline(kind: RevisionKind, t: Tx): ConfigRevision {
  return latestRevision(kind, t) ?? writeRevision(kind, "baseline", "system", t);
}

/** Call inside the transaction that made the change, after it. */
export function writeRevision(
  kind: RevisionKind,
  summary: string,
  actor: Actor,
  t: Tx,
  restoredFromId?: number,
): ConfigRevision {
  const latest = t
    .select({ v: max(schema.configRevisions.version) })
    .from(schema.configRevisions)
    .where(eq(schema.configRevisions.kind, kind))
    .get();
  const { actorId, actorName } = actorFields(actor);
  return t
    .insert(schema.configRevisions)
    .values({
      kind,
      version: (latest?.v ?? 0) + 1,
      snapshot: snapshotOf(kind, t),
      summary,
      actorId,
      actorName,
      restoredFromId: restoredFromId ?? null,
    })
    .returning()
    .get();
}

export function listRevisions(kind: RevisionKind, limit = 50): ConfigRevision[] {
  return db
    .select()
    .from(schema.configRevisions)
    .where(eq(schema.configRevisions.kind, kind))
    .orderBy(desc(schema.configRevisions.version))
    .limit(limit)
    .all();
}

export function getRevision(id: number): ConfigRevision | undefined {
  return db.select().from(schema.configRevisions).where(eq(schema.configRevisions.id, id)).get();
}

function applyFilterSnapshot(t: Tx, snapshot: FilterSnapshot) {
  t.delete(schema.blacklistTerms).run();
  for (const term of snapshot) {
    t.insert(schema.blacklistTerms)
      .values({ ...term, updatedAt: Date.now() })
      .run();
  }
}

function applySettingsSnapshot(t: Tx, snapshot: Settings) {
  t.delete(schema.settings).run();
  writeSettings({ ...DEFAULT_SETTINGS, ...snapshot }, t);
}

function applyStaffSnapshot(t: Tx, snapshot: StaffSnapshot) {
  // Roles and disabled flags only: people are never deleted by a restore, and a restore that would
  // leave no enabled owner is refused, since nobody could undo it.
  if (!snapshot.some((s) => s.role === "owner" && !s.disabled)) throw new DomainError("last_owner");
  const rows = t.select().from(schema.staff).all();
  const find = (s: StaffSnapshot[number]) =>
    rows.find((r) =>
      s.id !== undefined ? r.id === s.id : s.discordId !== null && r.discordId === s.discordId,
    );
  const signOut = (userId: number) =>
    t.delete(schema.authSessions).where(eq(schema.authSessions.userId, userId)).run();
  const named = new Set<number>();
  for (const s of snapshot) {
    const row = find(s);
    if (!row) continue;
    named.add(row.id);
    t.update(schema.staff)
      .set({ role: s.role as never, disabled: s.disabled, roleSource: s.roleSource as never })
      .where(eq(schema.staff.id, row.id))
      .run();
    if (s.disabled) signOut(row.userId);
  }
  // Someone added after the snapshot is disabled, which is what "as it was" means for access.
  for (const row of rows) {
    if (named.has(row.id) || row.disabled) continue;
    t.update(schema.staff).set({ disabled: true }).where(eq(schema.staff.id, row.id)).run();
    signOut(row.userId);
  }
}

export function restoreRevision(id: number, actor: Actor, revertOfId?: number): ConfigRevision {
  const target = getRevision(id);
  if (!target) throw new DomainError("not_found");
  return tx((t) => {
    const replaced = ensureBaseline(target.kind, t);
    if (target.kind === "chat_filter") applyFilterSnapshot(t, target.snapshot as FilterSnapshot);
    else if (target.kind === "settings") applySettingsSnapshot(t, target.snapshot as Settings);
    else applyStaffSnapshot(t, target.snapshot as StaffSnapshot);
    const revision = writeRevision(target.kind, `restore:${target.version}`, actor, t, target.id);
    recordAudit(
      actor,
      {
        action: `${target.kind}.restore`,
        targetType:
          target.kind === "chat_filter"
            ? "chat_filter"
            : target.kind === "staff"
              ? "staff"
              : "settings",
        targetId: target.kind,
        targetLabel: `v${target.version}`,
        before: { revisionId: replaced.id, version: replaced.version },
        after: {
          revisionId: revision.id,
          version: revision.version,
          restoredVersion: target.version,
        },
        revertible: true,
        revertOfId: revertOfId ?? null,
      },
      t,
    );
    return revision;
  });
}
