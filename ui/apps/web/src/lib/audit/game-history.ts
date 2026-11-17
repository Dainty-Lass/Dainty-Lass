import type { AuditEvent } from "@/lib/db/schema";
import type { BackendAuditRecord } from "@/lib/game/backend";

/**
 * The game server's own admin record for one player, as History rows: what its admin sign-in, the host kit and
 * anything else on /admin/v1 changed. Calls this dashboard made are signed as `manager:<id>` and already have
 * their own row, so they are left out. Rows get negative ids: they are read-only, with no Undo.
 */
export function gameHistory(
  records: BackendAuditRecord[],
  userId: string,
  name: string,
  controllerId: string,
): AuditEvent[] {
  return records
    .filter((r) => r.targetUserId === userId && !r.actorUserId?.startsWith("manager:"))
    .map((r, i) => ({
      id: -(i + 1),
      at: Date.parse(r.createdAt ?? "") || 0,
      actorId: null,
      actorName: r.actorUserId ?? "game",
      controllerId,
      action: r.action,
      targetType: "player",
      targetId: userId,
      targetLabel: name,
      reason: r.reason ?? null,
      before: r.oldState ?? null,
      after: r.newState ?? null,
      revertible: false,
      revertOfId: null,
      revertedById: null,
      ip: null,
    }));
}
