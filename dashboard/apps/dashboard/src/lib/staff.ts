import { and, asc, eq, ne } from "drizzle-orm";
import { type Actor, recordAudit } from "@/lib/audit/record";
import { config } from "@/lib/config";
import { db, schema, tx } from "@/lib/db";
import type { Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { ensureBaseline, writeRevision } from "@/lib/revisions";
import type { DiscordUser } from "./auth/discord";
import { canAssign, type Role } from "./auth/permissions";

export function listStaff(): Staff[] {
  return db
    .select()
    .from(schema.staff)
    .orderBy(asc(schema.staff.disabled), asc(schema.staff.username))
    .all();
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

export type SignInOutcome =
  | { ok: true; staff: Staff }
  | { ok: false; reason: "not_staff" | "disabled" };

/**
 * Decides who a Discord account is here. An owner listed in the environment always gets in; someone
 * added by hand keeps their role; someone whose access came from a guild role is re-read every time,
 * so taking the role away in Discord takes the dashboard away at their next sign-in.
 */
export function resolveSignIn(
  user: DiscordUser,
  guildRole: Role | null | undefined,
): SignInOutcome {
  return tx((t) => {
    const existing = t.select().from(schema.staff).where(eq(schema.staff.discordId, user.id)).get();
    const profile = { username: user.username, globalName: user.global_name, avatar: user.avatar };
    const envOwner = config.ownerDiscordIds.includes(user.id);

    if (existing) {
      if (existing.disabled) return { ok: false, reason: "disabled" };
      let role = existing.role;
      if (envOwner) role = "owner";
      else if (existing.roleSource === "guild") {
        if (!guildRole) return { ok: false, reason: "not_staff" };
        role = guildRole;
      }
      const staff = t
        .update(schema.staff)
        .set({ ...profile, role })
        .where(eq(schema.staff.id, existing.id))
        .returning()
        .get();
      if (role !== existing.role) {
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
            after: { role },
            reason: envOwner ? "owner_env" : "guild_role",
          },
          t,
        );
      }
      return { ok: true, staff };
    }

    const role: Role | null = envOwner ? "owner" : (guildRole ?? null);
    if (!role) return { ok: false, reason: "not_staff" };
    ensureBaseline("staff", t);
    const staff = t
      .insert(schema.staff)
      .values({ discordId: user.id, ...profile, role, roleSource: envOwner ? "manual" : "guild" })
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
        after: { role, roleSource: staff.roleSource },
        reason: envOwner ? "owner_env" : "guild_role",
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
    ensureBaseline("staff", t);
    const staff = t
      .insert(schema.staff)
      .values({
        discordId: input.discordId,
        username: input.username.trim().slice(0, 32) || input.discordId,
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

export function updateStaff(
  actor: (Actor & Staff) | Actor,
  id: number,
  change: { role?: Role; disabled?: boolean; note?: string | null },
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
      .set({ ...change, ...(change.role ? { roleSource: "manual" as const } : {}) })
      .where(eq(schema.staff.id, id))
      .returning()
      .get();
    if (after.disabled) t.delete(schema.sessions).where(eq(schema.sessions.staffId, id)).run();
    writeRevision("staff", `edit:${after.username}`, actor, t);
    recordAudit(
      actor,
      {
        action: "staff.update",
        targetType: "staff",
        targetId: id,
        targetLabel: after.username,
        before: { role: before.role, disabled: before.disabled, note: before.note },
        after: { role: after.role, disabled: after.disabled, note: after.note },
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
