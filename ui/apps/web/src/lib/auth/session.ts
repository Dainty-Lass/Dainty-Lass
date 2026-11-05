import { eq } from "drizzle-orm";
import { headers } from "next/headers";
import { cache } from "react";
import { db, schema } from "@/lib/db";
import type { Staff } from "@/lib/db/schema";

/** The signed-in member for this request, or null. Cached per request. */
export const currentStaff = cache(async (): Promise<Staff | null> => {
  const { auth } = await import("./server");
  const session = await auth.api.getSession({ headers: await headers() }).catch(() => null);
  if (!session) return null;
  const me = db
    .select()
    .from(schema.staff)
    .where(eq(schema.staff.userId, Number(session.user.id)))
    .get();
  if (!me || me.disabled) return null;
  // Touch at most once a minute, so page loads are not all writes.
  if (!me.lastSeenAt || Date.now() - me.lastSeenAt > 60_000)
    db.update(schema.staff).set({ lastSeenAt: Date.now() }).where(eq(schema.staff.id, me.id)).run();
  return me;
});

export function revokeSessionsFor(staffId: number): void {
  const me = db
    .select({ userId: schema.staff.userId })
    .from(schema.staff)
    .where(eq(schema.staff.id, staffId))
    .get();
  if (me) db.delete(schema.authSessions).where(eq(schema.authSessions.userId, me.userId)).run();
}

/**
 * The peer as the reverse proxy in front reports it. The last hop, the one that proxy appended: the first is
 * whatever the client sent. Only for the audit trail and rate limits, never for access.
 */
export function clientIp(h: Headers): string | null {
  return h.get("x-forwarded-for")?.split(",").at(-1)?.trim() || h.get("x-real-ip") || null;
}
