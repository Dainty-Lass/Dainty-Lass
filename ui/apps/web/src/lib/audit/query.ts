import { and, desc, eq, gte, inArray, like, lt, lte, or, type SQL, sql } from "drizzle-orm";
import { db, schema } from "@/lib/db";
import type { AuditEvent } from "@/lib/db/schema";

export interface AuditQuery {
  controllerIds?: string[];
  actorId?: number;
  targetType?: AuditEvent["targetType"];
  targetId?: string;
  /** A prefix: "player." matches every player action. */
  action?: string;
  text?: string;
  from?: number;
  to?: number;
  revertibleOnly?: boolean;
  page?: number;
  pageSize?: number;
}

export function queryAudit(q: AuditQuery): { events: AuditEvent[]; total: number } {
  const parts: SQL[] = [];
  if (q.actorId) parts.push(eq(schema.auditEvents.actorId, q.actorId));
  if (q.controllerIds) parts.push(inArray(schema.auditEvents.controllerId, q.controllerIds));
  if (q.targetType) parts.push(eq(schema.auditEvents.targetType, q.targetType));
  if (q.targetId) parts.push(eq(schema.auditEvents.targetId, q.targetId));
  // A plain prefix compare: LIKE would need "_" stripped or escaped, and actions such as chat.direct_read have one.
  if (q.action)
    parts.push(sql`substr(${schema.auditEvents.action}, 1, ${q.action.length}) = ${q.action}`);
  if (q.text) {
    const t = `%${q.text.replace(/[%_]/g, "")}%`;
    parts.push(
      or(
        like(schema.auditEvents.targetLabel, t),
        like(schema.auditEvents.reason, t),
        like(schema.auditEvents.actorName, t),
      )!,
    );
  }
  if (q.from) parts.push(gte(schema.auditEvents.at, q.from));
  if (q.to) parts.push(lte(schema.auditEvents.at, q.to));
  if (q.revertibleOnly)
    parts.push(
      and(
        eq(schema.auditEvents.revertible, true),
        sql`${schema.auditEvents.revertedById} is null`,
      )!,
    );
  const where = parts.length ? and(...parts) : undefined;
  const pageSize = Math.min(Math.max(q.pageSize ?? 50, 1), 500);
  const page = Math.max(q.page ?? 1, 1);
  const events = db
    .select()
    .from(schema.auditEvents)
    .where(where)
    .orderBy(desc(schema.auditEvents.at), desc(schema.auditEvents.id))
    .limit(pageSize)
    .offset((page - 1) * pageSize)
    .all();
  const total =
    db.select({ n: sql<number>`count(*)` }).from(schema.auditEvents).where(where).get()?.n ?? 0;
  return { events, total };
}

export function getAuditEvent(id: number): AuditEvent | undefined {
  return db.select().from(schema.auditEvents).where(eq(schema.auditEvents.id, id)).get();
}

export function recentAudit(limit = 8, controllerIds?: string[]): AuditEvent[] {
  return db
    .select()
    .from(schema.auditEvents)
    .where(controllerIds ? inArray(schema.auditEvents.controllerId, controllerIds) : undefined)
    .orderBy(desc(schema.auditEvents.id))
    .limit(limit)
    .all();
}

export function auditSince(since: number, controllerIds?: string[]): number {
  return (
    db
      .select({ n: sql<number>`count(*)` })
      .from(schema.auditEvents)
      .where(
        and(
          gte(schema.auditEvents.at, since),
          controllerIds ? inArray(schema.auditEvents.controllerId, controllerIds) : undefined,
        ),
      )
      .get()?.n ?? 0
  );
}

export function auditBefore(id: number): AuditEvent[] {
  return db
    .select()
    .from(schema.auditEvents)
    .where(lt(schema.auditEvents.id, id))
    .orderBy(desc(schema.auditEvents.id))
    .limit(1)
    .all();
}

export function auditActors(): { id: number; name: string }[] {
  return db
    .selectDistinct({ id: schema.auditEvents.actorId, name: schema.auditEvents.actorName })
    .from(schema.auditEvents)
    .where(sql`${schema.auditEvents.actorId} is not null`)
    .all()
    .map((r) => ({ id: r.id!, name: r.name }));
}
