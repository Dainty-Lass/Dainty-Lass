import { and, asc, desc, eq, inArray, or, type SQL, sql } from "drizzle-orm";
import { type Actor, recordAudit } from "@/lib/audit/record";
import { db, schema, tx } from "@/lib/db";
import type { Report, ReportNote, Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";

export type { ReportCategory, ReportStatus } from "./categories";
export { REPORT_CATEGORIES, REPORT_PRIORITIES, REPORT_STATUSES } from "./categories";

import type { ReportCategory, ReportStatus } from "./categories";

export interface ReportListItem extends Report {
  assigneeName: string | null;
  /** Other reports against the same player, any status: repeat offenders float up. */
  priorAgainstTarget: number;
}

export function listReports(q: {
  status?: ReportStatus | "active";
  targetUserId?: string;
  assigneeId?: number;
  limit?: number;
}): ReportListItem[] {
  const parts: SQL[] = [];
  if (q.status === "active") parts.push(inArray(schema.reports.status, ["open", "claimed"]));
  else if (q.status) parts.push(eq(schema.reports.status, q.status));
  if (q.targetUserId) parts.push(eq(schema.reports.targetUserId, q.targetUserId));
  if (q.assigneeId) parts.push(eq(schema.reports.assigneeId, q.assigneeId));
  const prior = db
    .select({ target: schema.reports.targetUserId, n: sql<number>`count(*)`.as("n") })
    .from(schema.reports)
    .groupBy(schema.reports.targetUserId)
    .as("prior");
  const rows = db
    .select({ report: schema.reports, assigneeName: schema.staff.username, n: prior.n })
    .from(schema.reports)
    .leftJoin(schema.staff, eq(schema.staff.id, schema.reports.assigneeId))
    .leftJoin(prior, eq(prior.target, schema.reports.targetUserId))
    .where(parts.length ? and(...parts) : undefined)
    // High priority first, then oldest first: a queue, not a feed.
    .orderBy(
      sql`case ${schema.reports.priority} when 'high' then 0 when 'normal' then 1 else 2 end`,
      q.status === "resolved" || q.status === "dismissed"
        ? desc(schema.reports.updatedAt)
        : asc(schema.reports.createdAt),
    )
    .limit(q.limit ?? 200)
    .all();
  return rows.map((r) => ({
    ...r.report,
    assigneeName: r.assigneeName,
    priorAgainstTarget: Math.max(0, (r.n ?? 1) - 1),
  }));
}

export function reportCounts(): Record<ReportStatus, number> {
  const rows = db
    .select({ status: schema.reports.status, n: sql<number>`count(*)` })
    .from(schema.reports)
    .groupBy(schema.reports.status)
    .all();
  const counts = { open: 0, claimed: 0, resolved: 0, dismissed: 0 };
  for (const r of rows) counts[r.status] = r.n;
  return counts;
}

export function getReport(
  id: number,
): (Report & { assignee: Staff | null; notes: ReportNote[] }) | undefined {
  const row = db
    .select({ report: schema.reports, assignee: schema.staff })
    .from(schema.reports)
    .leftJoin(schema.staff, eq(schema.staff.id, schema.reports.assigneeId))
    .where(eq(schema.reports.id, id))
    .get();
  if (!row) return undefined;
  const notes = db
    .select()
    .from(schema.reportNotes)
    .where(eq(schema.reportNotes.reportId, id))
    .orderBy(schema.reportNotes.at)
    .all();
  return { ...row.report, assignee: row.assignee, notes };
}

export interface NewReport {
  source: Report["source"];
  externalId?: string | null;
  reporter?: { userId?: string | null; name?: string | null } | null;
  target: { userId?: string | null; name: string };
  category: ReportCategory;
  description: string;
  priority?: Report["priority"];
  context?: Report["context"];
}

export function createReport(
  actor: Actor,
  input: NewReport,
): { report: Report; duplicate: boolean } {
  return tx((t) => {
    if (input.externalId) {
      const existing = t
        .select()
        .from(schema.reports)
        .where(
          and(
            eq(schema.reports.source, input.source),
            eq(schema.reports.externalId, input.externalId),
          ),
        )
        .get();
      if (existing) return { report: existing, duplicate: true };
    }
    // Cheating and hate speech jump the queue; everything else waits its turn.
    const priority =
      input.priority ??
      (input.category === "cheating" || input.category === "hate_speech" ? "high" : "normal");
    const report = t
      .insert(schema.reports)
      .values({
        source: input.source,
        externalId: input.externalId ?? null,
        reporterUserId: input.reporter?.userId ?? null,
        reporterName: input.reporter?.name ?? null,
        targetUserId: input.target.userId ?? null,
        targetName: input.target.name,
        category: input.category,
        description: input.description.slice(0, 2000),
        priority,
        context: input.context ?? null,
      })
      .returning()
      .get();
    recordAudit(
      actor,
      {
        action: "report.create",
        targetType: "report",
        targetId: report.id,
        targetLabel: report.targetName,
        after: report,
      },
      t,
    );
    return { report, duplicate: false };
  });
}

type Workflow = Pick<Report, "status" | "assigneeId" | "resolution" | "resolvedAt" | "priority">;

/** Every workflow change goes through here so each is audited with what it replaced, and so can be undone. */
export function updateReport(
  actor: Actor,
  id: number,
  change: Partial<Workflow>,
  opts: { reason?: string; revertOfId?: number } = {},
): Report {
  return tx((t) => {
    const before = t.select().from(schema.reports).where(eq(schema.reports.id, id)).get();
    if (!before) throw new DomainError("not_found");
    const after = t
      .update(schema.reports)
      .set({ ...change, updatedAt: Date.now() })
      .where(eq(schema.reports.id, id))
      .returning()
      .get();
    const pick = (r: Report): Workflow => ({
      status: r.status,
      assigneeId: r.assigneeId,
      resolution: r.resolution,
      resolvedAt: r.resolvedAt,
      priority: r.priority,
    });
    recordAudit(
      actor,
      {
        action: "report.update",
        targetType: "report",
        targetId: id,
        targetLabel: after.targetName,
        reason: opts.reason ?? null,
        before: pick(before),
        after: pick(after),
        revertible: true,
        revertOfId: opts.revertOfId,
      },
      t,
    );
    return after;
  });
}

export function claimReport(me: Staff, id: number): Report {
  const current = getReport(id);
  if (!current) throw new DomainError("not_found");
  if (current.status === "resolved" || current.status === "dismissed")
    throw new DomainError("conflict");
  return updateReport(me, id, { status: "claimed", assigneeId: me.id });
}

export function closeReport(
  me: Staff,
  id: number,
  status: "resolved" | "dismissed",
  resolution: string,
): Report {
  if (resolution.trim().length < 3) throw new DomainError("reason_required");
  return updateReport(me, id, {
    status,
    resolution: resolution.trim().slice(0, 1000),
    resolvedAt: Date.now(),
    assigneeId: getReport(id)?.assigneeId ?? me.id,
  });
}

export function reopenReport(me: Staff, id: number): Report {
  return updateReport(me, id, {
    status: "open",
    resolution: null,
    resolvedAt: null,
    assigneeId: null,
  });
}

export function addNote(me: Staff, reportId: number, body: string): ReportNote {
  const text = body.trim();
  if (text.length < 1 || text.length > 2000) throw new DomainError("invalid_input");
  return tx((t) => {
    if (
      !t
        .select({ id: schema.reports.id })
        .from(schema.reports)
        .where(eq(schema.reports.id, reportId))
        .get()
    )
      throw new DomainError("not_found");
    const note = t
      .insert(schema.reportNotes)
      .values({ reportId, staffId: me.id, authorName: me.globalName || me.username, body: text })
      .returning()
      .get();
    t.update(schema.reports)
      .set({ updatedAt: Date.now() })
      .where(eq(schema.reports.id, reportId))
      .run();
    recordAudit(
      me,
      { action: "report.note", targetType: "report", targetId: reportId, after: { body: text } },
      t,
    );
    return note;
  });
}

export function reportsAgainst(userId: string, name: string): Report[] {
  return db
    .select()
    .from(schema.reports)
    .where(or(eq(schema.reports.targetUserId, userId), eq(schema.reports.targetName, name)))
    .orderBy(desc(schema.reports.createdAt))
    .limit(50)
    .all();
}
