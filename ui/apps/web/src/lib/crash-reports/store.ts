import { desc, eq, getTableColumns, inArray, sql } from "drizzle-orm";
import { db, schema, tx } from "@/lib/db";
import type { CrashReport } from "@/lib/db/schema";
import type { NewCrashReport } from "./parse";

// A game server can't fill the dashboard's disk: the newest reports are kept, by count and by stored size.
export const KEEP_CRASH_REPORTS = 500;
export const KEEP_CRASH_BYTES = 256 * 1024 * 1024;

export type CrashReportListItem = Omit<CrashReport, "files"> & { fileCount: number };

export function listCrashReports(q: { controllerIds?: string[]; limit?: number } = {}) {
  const { files: _files, ...cols } = getTableColumns(schema.crashReports);
  return db
    .select({ ...cols, fileCount: sql<number>`json_array_length(${schema.crashReports.files})` })
    .from(schema.crashReports)
    .where(q.controllerIds ? inArray(schema.crashReports.controllerId, q.controllerIds) : undefined)
    .orderBy(desc(schema.crashReports.receivedAt))
    .limit(q.limit ?? 200)
    .all() as CrashReportListItem[];
}

export function getCrashReport(id: string): CrashReport | undefined {
  return db.select().from(schema.crashReports).where(eq(schema.crashReports.id, id)).get();
}

export function createCrashReport(
  input: NewCrashReport,
  controllerId = "local",
): { report: CrashReport; duplicate: boolean } {
  return tx((t) => {
    const existing = t
      .select()
      .from(schema.crashReports)
      .where(eq(schema.crashReports.id, input.id))
      .get();
    if (existing) return { report: existing, duplicate: true };
    const report = t
      .insert(schema.crashReports)
      .values({
        id: input.id,
        controllerId,
        receivedAt: input.receivedAt,
        userId: input.account.userId,
        userName: input.account.name,
        crashId: input.crashId,
        gameVersion: input.gameVersion,
        launcherVersion: input.launcherVersion,
        exitCode: input.exitCode,
        serverPath: input.serverPath,
        files: input.files,
      })
      .returning()
      .get();
    // Pruned by our own clock: receivedAt is the sender's and could pin a report forever.
    const sizes = t
      .select({
        id: schema.crashReports.id,
        bytes: sql<number>`length(${schema.crashReports.files})`,
      })
      .from(schema.crashReports)
      .orderBy(desc(schema.crashReports.createdAt), desc(sql`rowid`))
      .all();
    let total = 0;
    const drop: string[] = [];
    sizes.forEach((row, i) => {
      total += row.bytes;
      if (row.id !== report.id && (i >= KEEP_CRASH_REPORTS || total > KEEP_CRASH_BYTES))
        drop.push(row.id);
    });
    for (let i = 0; i < drop.length; i += 500)
      t.delete(schema.crashReports)
        .where(inArray(schema.crashReports.id, drop.slice(i, i + 500)))
        .run();
    return { report, duplicate: false };
  });
}
