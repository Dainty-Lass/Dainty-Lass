import { db, schema } from "@/lib/db";
import type { Staff } from "@/lib/db/schema";

export type Actor = Pick<Staff, "id" | "username" | "globalName"> | "system" | { ingest: string };

export interface AuditInput {
  action: string;
  targetType:
    | "player"
    | "report"
    | "chat_filter"
    | "filter_term"
    | "staff"
    | "settings"
    | "session";
  targetId?: string | number | null;
  targetLabel?: string | null;
  reason?: string | null;
  before?: unknown;
  after?: unknown;
  /** True only when revert.ts holds an inverse for `action`. */
  revertible?: boolean;
  revertOfId?: number | null;
  ip?: string | null;
}

export function actorFields(actor: Actor): { actorId: number | null; actorName: string } {
  if (actor === "system") return { actorId: null, actorName: "system" };
  if ("ingest" in actor) return { actorId: null, actorName: `ingest:${actor.ingest}` };
  return { actorId: actor.id, actorName: actor.globalName || actor.username };
}

/**
 * Appends one event. Pass the transaction the change ran in, so the change and its record commit
 * together; a change that reached the game first is recorded after it answered.
 */
export function recordAudit(
  actor: Actor,
  input: AuditInput,
  t: Pick<typeof db, "insert" | "update"> = db,
): number {
  const row = t
    .insert(schema.auditEvents)
    .values({
      ...actorFields(actor),
      action: input.action,
      targetType: input.targetType,
      targetId: input.targetId == null ? null : String(input.targetId),
      targetLabel: input.targetLabel ?? null,
      reason: input.reason ?? null,
      before: input.before ?? null,
      after: input.after ?? null,
      revertible: input.revertible ?? false,
      revertOfId: input.revertOfId ?? null,
      ip: input.ip ?? null,
    })
    .returning({ id: schema.auditEvents.id })
    .get();
  return row.id;
}
