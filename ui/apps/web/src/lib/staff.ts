import { and, asc, eq, ne } from "drizzle-orm";
import { type Actor, recordAudit } from "@/lib/audit/record";
import { config } from "@/lib/config";
import { db, schema, tx } from "@/lib/db";
import type { Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { ensureBaseline, writeRevision } from "@/lib/revisions";
import type { GuildStanding } from "./auth/discord";
import { canAssign, type Role } from "./auth/permissions";

export function listStaff(): Staff[] {
  return db
    .select()
    .from(schema.staff)
    .orderBy(asc(schema.staff.disabled), asc(schema.staff.username))
    .all();
}

/** Each member with a linked Discord account, by that id: names the game's record of what they did. */
export function staffByDiscord(): Map<string, string> {
  return new Map(
    listStaff()
      .filter((s) => s.discordId)
      .map((s) => [s.discordId as string, s.displayName || s.username]),
  );
}

export function getStaff(id: number): Staff | undefined {
  return db.select().from(schema.staff).where(eq(schema.staff.id, id)).get();
}

type Tx = Parameters<Parameters<typeof db.transaction>[0]>[0];

function enabledOwners(t: Pick<Tx, "select">, except?: number): number {
  return t
    .select({ id: schema.staff.id })
    .from(schema.staff)
    .where(
      and(
        eq(schema.staff.role, "owner"),
        eq(schema.staff.disabled, false),
        except ? ne(schema.staff.id, except) : undefined,
      ),
    )
    .all().length;
}

export type SignInOutcome = { ok: true; staff: Staff } | { ok: false; reason: "disabled" };

/** A sign-in identity as Better Auth holds it, with the Discord account linked to it, if any. */
export interface SignInIdentity {
  userId: number;
  name: string;
  image: string | null;
  /** Only a confirmed address: it can make an owner. */
  email: string | null;
  discordId: string | null;
}

/**
 * Decides who a sign-in identity is here. Anyone may sign in; only a disabled account is refused. The tier:
 * an owner listed in the environment is an owner; a role set by hand here is kept; otherwise the highest
 * role the guild maps, re-read every time, so a role taken away in Discord takes the tier with it; and
 * with no such role, a plain user. `guild` undefined means it could not be read, and the last tier stands.
 * Holding the guild's approved role approves the account.
 */
export function resolveSignIn(
  user: SignInIdentity,
  guild: GuildStanding | null | undefined,
): SignInOutcome {
  return tx((t) => {
    const existing = t
      .select()
      .from(schema.staff)
      .where(eq(schema.staff.userId, user.userId))
      .get();
    if (existing?.disabled) return { ok: false, reason: "disabled" };
    const profile = {
      userId: user.userId,
      username: user.name.slice(0, 64),
      avatar: user.image,
      ...(user.discordId ? { discordId: user.discordId } : {}),
    };
    const envOwner =
      (user.discordId !== null && config.ownerDiscordIds.includes(user.discordId)) ||
      (user.email !== null && config.ownerEmails.includes(user.email));

    const fromGuild: { role: Role; roleSource: "guild" | "default" } = guild?.role
      ? { role: guild.role, roleSource: "guild" }
      : { role: "user", roleSource: "default" };
    const tier: { role: Role; roleSource: Staff["roleSource"] } = envOwner
      ? { role: "owner", roleSource: "manual" }
      : existing?.roleSource === "manual" || (existing && guild === undefined)
        ? { role: existing.role, roleSource: existing.roleSource }
        : fromGuild;
    const approval =
      guild?.approved && !existing?.approvedAt
        ? { approvedAt: Date.now(), approvedVia: "guild" as const }
        : {};

    if (existing) {
      const staff = t
        .update(schema.staff)
        .set({ ...profile, ...tier, ...approval })
        .where(eq(schema.staff.id, existing.id))
        .returning()
        .get();
      if (tier.role !== existing.role) {
        ensureBaseline("staff", t);
        writeRevision("staff", `role:${staff.username}`, "system", t);
        recordAudit(
          "system",
          {
            action: "staff.update",
            targetType: "staff",
            targetId: staff.id,
            targetLabel: staff.username,
            before: { role: existing.role },
            after: { role: tier.role },
            reason: envOwner ? "owner_env" : "guild_role",
          },
          t,
        );
      }
      return { ok: true, staff };
    }

    ensureBaseline("staff", t);
    const staff = t
      .insert(schema.staff)
      .values({ ...profile, ...tier, ...approval })
      .returning()
      .get();
    writeRevision("staff", `add:${staff.username}`, "system", t);
    recordAudit(
      "system",
      {
        action: "staff.create",
        targetType: "staff",
        targetId: staff.id,
        targetLabel: staff.username,
        after: {
          role: staff.role,
          roleSource: staff.roleSource,
          approved: Boolean(staff.approvedAt),
        },
        reason: envOwner
          ? "owner_env"
          : staff.roleSource === "guild"
            ? "guild_role"
            : "first_sign_in",
      },
      t,
    );
    return { ok: true, staff };
  });
}

export function addStaff(
  actor: Staff,
  input: { discordId: string; username: string; role: Role; note?: string },
): Staff {
  if (!/^\d{17,20}$/.test(input.discordId)) throw new DomainError("invalid_input");
  if (!canAssign(actor.role, input.role)) throw new DomainError("forbidden");
  return tx((t) => {
    if (t.select().from(schema.staff).where(eq(schema.staff.discordId, input.discordId)).get())
      throw new DomainError("conflict");
    if (
      t
        .select()
        .from(schema.authAccounts)
        .where(
          and(
            eq(schema.authAccounts.providerId, "discord"),
            eq(schema.authAccounts.accountId, input.discordId),
          ),
        )
        .get()
    )
      throw new DomainError("conflict");
    ensureBaseline("staff", t);
    const username = input.username.trim().slice(0, 32) || input.discordId;
    // A sign-in identity with their Discord linked, so their first Discord sign-in lands on this row. The
    // address is a placeholder until Discord hands over the real one.
    const now = new Date();
    const user = t
      .insert(schema.authUsers)
      .values({
        name: username,
        email: `discord-${input.discordId}@users.invalid`,
        createdAt: now,
        updatedAt: now,
      })
      .returning()
      .get();
    t.insert(schema.authAccounts)
      .values({
        userId: user.id,
        providerId: "discord",
        accountId: input.discordId,
        createdAt: now,
        updatedAt: now,
      })
      .run();
    const staff = t
      .insert(schema.staff)
      .values({
        userId: user.id,
        discordId: input.discordId,
        username,
        role: input.role,
        note: input.note?.trim() || null,
      })
      .returning()
      .get();
    writeRevision("staff", `add:${staff.username}`, actor, t);
    recordAudit(
      actor,
      {
        action: "staff.create",
        targetType: "staff",
        targetId: staff.id,
        targetLabel: staff.username,
        after: { role: staff.role, disabled: false },
        revertible: true,
      },
      t,
    );
    return staff;
  });
}

/** What an admin may change on an account, and what a revert puts back. */
export interface MemberChange {
  role?: Role;
  roleSource?: Staff["roleSource"];
  disabled?: boolean;
  note?: string | null;
  approvedAt?: number | null;
  approvedVia?: Staff["approvedVia"];
}

const pick = (s: Staff): Required<MemberChange> => ({
  role: s.role,
  roleSource: s.roleSource,
  disabled: s.disabled,
  note: s.note,
  approvedAt: s.approvedAt,
  approvedVia: s.approvedVia,
});

export function updateStaff(
  actor: (Actor & Staff) | Actor,
  id: number,
  change: MemberChange,
  revertOfId?: number,
): Staff {
  return tx((t) => {
    const before = t.select().from(schema.staff).where(eq(schema.staff.id, id)).get();
    if (!before) throw new DomainError("not_found");
    if (typeof actor === "object" && "role" in actor) {
      if (actor.id === id && (change.disabled || (change.role && change.role !== before.role)))
        throw new DomainError("self_lockout");
      if (change.role && !canAssign(actor.role, change.role)) throw new DomainError("forbidden");
      // Nobody below owner touches an owner.
      if (before.role === "owner" && actor.role !== "owner") throw new DomainError("forbidden");
    }
    const losingOwner =
      before.role === "owner" &&
      !before.disabled &&
      ((change.role && change.role !== "owner") || change.disabled);
    if (losingOwner && enabledOwners(t, id) === 0) throw new DomainError("last_owner");
    ensureBaseline("staff", t);
    const after = t
      .update(schema.staff)
      // A role set here sticks; one left to Discord is re-read at the next sign-in.
      .set({
        ...change,
        ...(change.role && !change.roleSource ? { roleSource: "manual" as const } : {}),
      })
      .where(eq(schema.staff.id, id))
      .returning()
      .get();
    if (after.disabled)
      t.delete(schema.authSessions).where(eq(schema.authSessions.userId, after.userId)).run();
    writeRevision("staff", `edit:${after.username}`, actor, t);
    recordAudit(
      actor,
      {
        action: "staff.update",
        targetType: "staff",
        targetId: id,
        targetLabel: after.username,
        before: pick(before),
        after: pick(after),
        revertible: true,
        revertOfId,
      },
      t,
    );
    return after;
  });
}

export function hasAnyStaff(): boolean {
  return Boolean(db.select({ id: schema.staff.id }).from(schema.staff).limit(1).get());
}
