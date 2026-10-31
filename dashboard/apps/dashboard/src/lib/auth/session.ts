import { createHash, randomBytes } from "node:crypto";
import { and, eq, gt, lt } from "drizzle-orm";
import { cookies, headers } from "next/headers";
import { cache } from "react";
import { config, secureCookies } from "@/lib/config";
import { db, schema } from "@/lib/db";
import type { Staff } from "@/lib/db/schema";

// __Host- on HTTPS pins the cookie to this origin and path; plain HTTP (a dev box) cannot use the prefix.
export const SESSION_COOKIE = secureCookies() ? "__Host-dainty-session" : "dainty-session";

const hash = (token: string) => createHash("sha256").update(token).digest("hex");

export async function createSession(
  staffId: number,
): Promise<{ token: string; expiresAt: number }> {
  const token = randomBytes(32).toString("base64url");
  const expiresAt = Date.now() + config.sessionDays * 86_400_000;
  const h = await headers();
  db.insert(schema.sessions)
    .values({
      tokenHash: hash(token),
      staffId,
      expiresAt,
      ip: clientIp(h),
      userAgent: h.get("user-agent")?.slice(0, 300) ?? null,
    })
    .run();
  db.update(schema.staff).set({ lastSeenAt: Date.now() }).where(eq(schema.staff.id, staffId)).run();
  // Expired sessions go whenever someone signs in; nothing else needs a timer.
  db.delete(schema.sessions).where(lt(schema.sessions.expiresAt, Date.now())).run();
  return { token, expiresAt };
}

export function sessionCookie(token: string, expiresAt: number) {
  return {
    name: SESSION_COOKIE,
    value: token,
    httpOnly: true,
    secure: secureCookies(),
    sameSite: "lax" as const,
    path: "/",
    expires: new Date(expiresAt),
  };
}

/** The signed-in staff member for this request, or null. Cached per request. */
export const currentStaff = cache(async (): Promise<Staff | null> => {
  const token = (await cookies()).get(SESSION_COOKIE)?.value;
  if (!token) return null;
  const row = db
    .select({ staff: schema.staff, session: schema.sessions })
    .from(schema.sessions)
    .innerJoin(schema.staff, eq(schema.staff.id, schema.sessions.staffId))
    .where(
      and(eq(schema.sessions.tokenHash, hash(token)), gt(schema.sessions.expiresAt, Date.now())),
    )
    .get();
  if (!row || row.staff.disabled) return null;
  // Touch at most once a minute, so page loads are not all writes.
  if (Date.now() - row.session.lastUsedAt > 60_000) {
    db.update(schema.sessions)
      .set({ lastUsedAt: Date.now() })
      .where(eq(schema.sessions.tokenHash, row.session.tokenHash))
      .run();
    db.update(schema.staff)
      .set({ lastSeenAt: Date.now() })
      .where(eq(schema.staff.id, row.staff.id))
      .run();
  }
  return row.staff;
});

export async function destroyCurrentSession(): Promise<void> {
  const token = (await cookies()).get(SESSION_COOKIE)?.value;
  if (token)
    db.delete(schema.sessions)
      .where(eq(schema.sessions.tokenHash, hash(token)))
      .run();
}

export function revokeSessionsFor(staffId: number): void {
  db.delete(schema.sessions).where(eq(schema.sessions.staffId, staffId)).run();
}

/** The peer as the reverse proxy in front reports it; only for the audit trail, never for access. */
export function clientIp(h: Headers): string | null {
  return h.get("x-forwarded-for")?.split(",")[0]?.trim() || h.get("x-real-ip") || null;
}
