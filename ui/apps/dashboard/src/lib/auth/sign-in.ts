import { and, eq } from "drizzle-orm";
import { recordAudit } from "@/lib/audit/record";
import { emailSignInAvailable } from "@/lib/config";
import { db, schema, tx } from "@/lib/db";
import type { Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { getSettings } from "@/lib/settings";
import { resolveSignIn, type SignInOutcome } from "@/lib/staff";
import { fetchGuildRoles, type GuildStanding, standingFromGuild } from "./discord";

/** How a session started, from the Better Auth endpoint that started it; kept in the audit trail. */
export function signInMethod(path: string): string {
  if (path.startsWith("/callback/")) return path.slice("/callback/".length);
  if (path.includes("email-otp")) return "email";
  if (path.startsWith("/passkey/")) return "passkey";
  if (path.startsWith("/dev/")) return "dev";
  return "other";
}

/**
 * The guild's say, read with the person's own Discord token. undefined when it cannot be read (no guild
 * set, no Discord linked, Discord down): the tier it gave last time stands. null when they are not in it.
 */
async function guildStanding(userId: number): Promise<GuildStanding | null | undefined> {
  const s = getSettings();
  if (!s.discordGuildId) return undefined;
  const linked = db
    .select({ id: schema.authAccounts.id })
    .from(schema.authAccounts)
    .where(
      and(eq(schema.authAccounts.userId, userId), eq(schema.authAccounts.providerId, "discord")),
    )
    .get();
  if (!linked) return undefined;
  try {
    // Imported here: the auth instance's own session hook is what calls this.
    const { auth } = await import("./server");
    const { accessToken } = await auth.api.getAccessToken({
      body: { accountId: String(linked.id), userId: String(userId) },
    });
    if (!accessToken) return undefined;
    return standingFromGuild(
      await fetchGuildRoles(accessToken, s.discordGuildId),
      s.discordRoleMap,
      s.discordApprovedRoleId,
    );
  } catch (error) {
    console.warn(
      "[auth] Reading guild roles failed:",
      error instanceof Error ? error.message : "unknown",
    );
    return undefined;
  }
}

/** Called for every new session, however it started: who the person is here, or why they may not enter. */
export async function signInMember(userId: number, path: string): Promise<SignInOutcome> {
  const user = db.select().from(schema.authUsers).where(eq(schema.authUsers.id, userId)).get();
  if (!user) return { ok: false, reason: "disabled" };
  const discord = db
    .select({ accountId: schema.authAccounts.accountId })
    .from(schema.authAccounts)
    .where(
      and(eq(schema.authAccounts.userId, userId), eq(schema.authAccounts.providerId, "discord")),
    )
    .get();
  const outcome = resolveSignIn(
    {
      userId,
      name: user.name || user.email.split("@")[0] || user.email,
      image: user.image,
      email: user.emailVerified ? user.email.toLowerCase() : null,
      discordId: discord?.accountId ?? null,
    },
    await guildStanding(userId),
  );
  if (outcome.ok)
    recordAudit(outcome.staff, {
      action: "session.sign_in",
      targetType: "session",
      targetId: outcome.staff.id,
      targetLabel: signInMethod(path),
    });
  return outcome;
}

export function refuseSignIn(userId: number, reason: string): void {
  const member = db.select().from(schema.staff).where(eq(schema.staff.userId, userId)).get();
  recordAudit("system", {
    action: "session.refused",
    targetType: "session",
    targetId: member?.id ?? userId,
    targetLabel: member?.username ?? String(userId),
    reason,
  });
}

/**
 * A provider added to an identity. Its first sign-in has no member yet (the session hook makes one); a
 * link from the Account page does, and a Discord link gives the member the id guild roles and the game use.
 */
export function accountLinked(userId: number, providerId: string, accountId: string): void {
  const member = db.select().from(schema.staff).where(eq(schema.staff.userId, userId)).get();
  if (!member) return;
  if (providerId === "discord")
    db.update(schema.staff)
      .set({ discordId: accountId })
      .where(eq(schema.staff.id, member.id))
      .run();
  recordAudit(member, {
    action: "account.linked",
    targetType: "staff",
    targetId: member.id,
    targetLabel: member.username,
    after: { provider: providerId },
  });
}

// The Better Auth endpoints that change how someone signs in, and what the audit trail calls each.
const CREDENTIAL_PATHS: Record<string, string> = {
  "/passkey/verify-registration": "passkey.added",
  "/passkey/delete-passkey": "passkey.removed",
};

export function auditCredentialChange(userId: number, path: string): void {
  const action = CREDENTIAL_PATHS[path];
  if (!action) return;
  const member = db.select().from(schema.staff).where(eq(schema.staff.userId, userId)).get();
  if (!member) return;
  recordAudit(member, {
    action,
    targetType: "staff",
    targetId: member.id,
    targetLabel: member.username,
  });
}

/**
 * Removes a linked provider. Here rather than Better Auth's endpoint so the last way in is kept, the
 * Discord id goes with Discord, and the audit trail knows which provider it was.
 */
export function unlinkAccount(me: Staff, accountRowId: number): void {
  tx((t) => {
    const account = t
      .select()
      .from(schema.authAccounts)
      .where(
        and(eq(schema.authAccounts.id, accountRowId), eq(schema.authAccounts.userId, me.userId)),
      )
      .get();
    if (!account) throw new DomainError("not_found");
    const others = waysIn(t, me.userId) - 1;
    if (others < 1) throw new DomainError("last_sign_in");
    t.delete(schema.authAccounts).where(eq(schema.authAccounts.id, account.id)).run();
    if (account.providerId === "discord")
      t.update(schema.staff).set({ discordId: null }).where(eq(schema.staff.id, me.id)).run();
    recordAudit(
      me,
      {
        action: "account.unlinked",
        targetType: "staff",
        targetId: me.id,
        targetLabel: me.username,
        before: { provider: account.providerId },
      },
      t,
    );
  });
}

/** How many ways someone has to sign in: providers, passkeys, and an address codes can reach. */
export function waysIn(t: Pick<typeof db, "select">, userId: number): number {
  const count = (table: typeof schema.authAccounts | typeof schema.authPasskeys) =>
    t.select({ id: table.id }).from(table).where(eq(table.userId, userId)).all().length;
  return count(schema.authAccounts) + count(schema.authPasskeys) + (emailWayIn(t, userId) ? 1 : 0);
}

/** A confirmed real address counts as a way in only where codes can be sent. */
function emailWayIn(t: Pick<typeof db, "select">, userId: number): boolean {
  if (!emailSignInAvailable()) return false;
  const user = t.select().from(schema.authUsers).where(eq(schema.authUsers.id, userId)).get();
  return Boolean(user?.emailVerified && !user.email.endsWith(".invalid"));
}
