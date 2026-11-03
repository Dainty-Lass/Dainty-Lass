import { eq } from "drizzle-orm";
import { deleteTerm, recreateTerm, updateTerm } from "@/lib/chat/terms";
import { db, schema } from "@/lib/db";
import type { AuditEvent, BlacklistTerm, Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { updateController } from "@/lib/fleet/controllers";
import type { Restriction } from "@/lib/game/admin-api";
import {
  banPlayer,
  liftTimeout,
  setPlayerAccess,
  setPlayerRoles,
  timeoutPlayer,
  unbanPlayer,
} from "@/lib/players";
import { updateReport } from "@/lib/reports/store";
import { restoreRevision } from "@/lib/revisions";
import { type MemberChange, updateStaff } from "@/lib/staff";
import { getAuditEvent } from "./query";

/**
 * Undo, one action at a time: each revertible action names the call that puts back its `before`. The
 * undo is itself an audited change pointing at the event it reverts (revertOfId), so it too shows in
 * the log and can be reverted in turn.
 */

type PlayerState = { status: string; restriction: Restriction | null } | null;

async function restoreRestriction(me: Staff, ev: AuditEvent) {
  const before = ev.before as PlayerState;
  const after = ev.after as PlayerState;
  const name = ev.targetLabel ?? "";
  const was = before?.restriction;
  const now = after?.restriction;
  if (!was || was.kind === "DISABLED") {
    // It was unrestricted: lift whatever this action added.
    if (now?.kind === "TIMEOUT") return liftTimeout(me, ev.controllerId, name, ev.id);
    if (now?.kind === "BAN") return unbanPlayer(me, ev.controllerId, name, "revert", ev.id);
    throw new DomainError("not_revertible");
  }
  if (was.kind === "BAN")
    return banPlayer(me, ev.controllerId, name, was.reason || "Restored by revert", ev.id);
  if (was.kind === "TIMEOUT") {
    // A timeout that has run out since is not reinstated: there is nothing left of it to put back.
    if (!was.until || Date.parse(was.until) <= Date.now() + 60_000)
      throw new DomainError("not_revertible");
    return timeoutPlayer(
      me,
      ev.controllerId,
      name,
      { until: was.until },
      was.reason || "Restored by revert",
      ev.id,
    );
  }
  throw new DomainError("not_revertible");
}

const REVERTERS: Record<string, (me: Staff, ev: AuditEvent) => Promise<unknown> | unknown> = {
  "player.ban": restoreRestriction,
  "player.timeout": restoreRestriction,
  "player.unban": restoreRestriction,
  "player.lift_timeout": restoreRestriction,
  "player.access": (me, ev) => {
    const before = ev.before as {
      status: "active" | "disabled" | "banned";
      approvalStatus: "pending" | "approved" | "rejected";
    };
    return setPlayerAccess(
      me,
      ev.controllerId,
      ev.targetId!,
      before,
      `Revert of change #${ev.id}`,
      ev.id,
    );
  },
  "player.roles": (me, ev) =>
    setPlayerRoles(
      me,
      ev.controllerId,
      ev.targetId!,
      (ev.before as { roles: string[] }).roles,
      `Revert of change #${ev.id}`,
      ev.id,
    ),
  "filter.term.create": (me, ev) => deleteTerm(me, Number(ev.targetId), ev.id),
  "filter.term.update": (me, ev) => {
    const b = ev.before as BlacklistTerm;
    return updateTerm(
      me,
      b.id,
      {
        pattern: b.pattern,
        matchType: b.matchType,
        action: b.action,
        category: b.category,
        note: b.note,
        enabled: b.enabled,
        channels: b.channels,
      },
      ev.id,
    );
  },
  "filter.term.delete": (me, ev) => recreateTerm(me, ev.before as BlacklistTerm, ev.id),
  "chat_filter.restore": (me, ev) =>
    restoreRevision((ev.before as { revisionId: number }).revisionId, me, ev.id),
  "settings.restore": (me, ev) =>
    restoreRevision((ev.before as { revisionId: number }).revisionId, me, ev.id),
  "staff.restore": (me, ev) =>
    restoreRevision((ev.before as { revisionId: number }).revisionId, me, ev.id),
  "settings.update": (me, ev) =>
    restoreRevision((ev.before as { revisionId: number }).revisionId, me, ev.id),
  "staff.create": (me, ev) => updateStaff(me, Number(ev.targetId), { disabled: true }, ev.id),
  "controller.update": (me, ev) => {
    const b = ev.before as {
      name: string;
      region: string | null;
      note: string | null;
      disabled: boolean;
    };
    return updateController(me, ev.controllerId, b, ev.id);
  },
  "staff.update": (me, ev) => {
    // Older events carry only role, disabled and note; what they lack stays as it is.
    const b = ev.before as MemberChange;
    return updateStaff(me, Number(ev.targetId), b, ev.id);
  },
  "report.update": (me, ev) => {
    const b = ev.before as Pick<AuditEvent, never> & {
      status: "open" | "claimed" | "resolved" | "dismissed";
      assigneeId: number | null;
      resolution: string | null;
      resolvedAt: number | null;
      priority: "low" | "normal" | "high";
    };
    return updateReport(me, Number(ev.targetId), b, {
      reason: `revert:${ev.id}`,
      revertOfId: ev.id,
    });
  },
};

export function canRevert(ev: AuditEvent): boolean {
  return ev.revertible && !ev.revertedById && ev.action in REVERTERS;
}

export async function revertEvent(me: Staff, id: number): Promise<number> {
  const ev = getAuditEvent(id);
  if (!ev) throw new DomainError("not_found");
  if (ev.revertedById) throw new DomainError("already_reverted");
  const revert = REVERTERS[ev.action];
  if (!ev.revertible || !revert) throw new DomainError("not_revertible");
  const latestBefore =
    db
      .select({ id: schema.auditEvents.id })
      .from(schema.auditEvents)
      .orderBy(schema.auditEvents.id)
      .all()
      .at(-1)?.id ?? 0;
  await revert(me, ev);
  // The reverter recorded its own event with revertOfId; link back to it.
  const undo = db
    .select({ id: schema.auditEvents.id })
    .from(schema.auditEvents)
    .where(eq(schema.auditEvents.revertOfId, id))
    .all()
    .filter((r) => r.id > latestBefore)
    .at(-1);
  if (undo)
    db.update(schema.auditEvents)
      .set({ revertedById: undo.id })
      .where(eq(schema.auditEvents.id, id))
      .run();
  return undo?.id ?? 0;
}
