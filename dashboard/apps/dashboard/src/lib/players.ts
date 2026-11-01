import { recordAudit } from "@/lib/audit/record";
import type { Staff } from "@/lib/db/schema";
import { DomainError, type ErrorCode } from "@/lib/errors";
import { fleet } from "@/lib/fleet/status";
import { adminApi, type GamePlayer } from "@/lib/game/admin-api";
import { type BackendAccount, backend } from "@/lib/game/backend";

/**
 * Moderation as the game enforces it, on one controller (each has its own accounts). The game answers
 * first; the audit event is written after, tagged with the controller, with the player's restriction
 * before and after, which is what a revert puts back.
 */

export const op = (me: Staff) => ({ id: me.discordId, name: me.globalName || me.username });

function restrictionOf(p: GamePlayer | null) {
  return p ? { status: p.status, restriction: p.restriction } : null;
}

function reasonOf(reason: string): string {
  const r = reason.trim().replace(/\s+/g, " ");
  // The admin API's rule: 3 to 400 characters on one line.
  if (r.length < 3 || r.length > 400) throw new DomainError("reason_required");
  return r;
}

async function current(controllerId: string, username: string, me: Staff): Promise<GamePlayer> {
  const p = await adminApi(controllerId).player(username, me.discordId);
  if (!p) throw new DomainError("not_found");
  return p;
}

export async function kickPlayer(
  me: Staff,
  controllerId: string,
  username: string,
  reason: string | null,
) {
  const before = await current(controllerId, username, me);
  const result = await adminApi(controllerId).kick(before.username, me.discordId);
  recordAudit(me, {
    action: "player.kick",
    targetType: "player",
    targetId: before.userId,
    targetLabel: before.username,
    controllerId,
    reason,
    after: { connectionsClosed: result.connectionsClosed },
  });
  return result;
}

export async function timeoutPlayer(
  me: Staff,
  controllerId: string,
  username: string,
  until: { minutes?: number; until?: string },
  reason: string,
  revertOfId?: number,
) {
  const before = await current(controllerId, username, me);
  const after = await adminApi(controllerId).timeout(
    before.username,
    until,
    reasonOf(reason),
    me.discordId,
  );
  recordAudit(me, {
    action: "player.timeout",
    targetType: "player",
    targetId: after.userId,
    targetLabel: after.username,
    controllerId,
    reason,
    before: restrictionOf(before),
    after: restrictionOf(after),
    revertible: true,
    revertOfId,
  });
  return after;
}

export async function banPlayer(
  me: Staff,
  controllerId: string,
  username: string,
  reason: string,
  revertOfId?: number,
) {
  const before = await current(controllerId, username, me);
  const after = await adminApi(controllerId).ban(before.username, reasonOf(reason), me.discordId);
  recordAudit(me, {
    action: "player.ban",
    targetType: "player",
    targetId: after.userId,
    targetLabel: after.username,
    controllerId,
    reason,
    before: restrictionOf(before),
    after: restrictionOf(after),
    revertible: true,
    revertOfId,
  });
  return after;
}

export async function unbanPlayer(
  me: Staff,
  controllerId: string,
  username: string,
  reason: string | null,
  revertOfId?: number,
) {
  const before = await current(controllerId, username, me);
  const after = await adminApi(controllerId).unban(
    before.username,
    reason?.trim() || null,
    me.discordId,
  );
  recordAudit(me, {
    action: "player.unban",
    targetType: "player",
    targetId: after.userId,
    targetLabel: after.username,
    controllerId,
    reason,
    before: restrictionOf(before),
    after: restrictionOf(after),
    revertible: true,
    revertOfId,
  });
  return after;
}

export async function liftTimeout(
  me: Staff,
  controllerId: string,
  username: string,
  revertOfId?: number,
) {
  const before = await current(controllerId, username, me);
  const after = await adminApi(controllerId).liftTimeout(before.username, me.discordId);
  recordAudit(me, {
    action: "player.lift_timeout",
    targetType: "player",
    targetId: after.userId,
    targetLabel: after.username,
    controllerId,
    before: restrictionOf(before),
    after: restrictionOf(after),
    revertible: true,
    revertOfId,
  });
  return after;
}

export interface FleetOutcome {
  controllerId: string;
  name: string;
  result: "done" | "absent" | "skipped" | "failed";
  code?: ErrorCode;
}

/**
 * The same name banned on every controller that can be reached: a player banned on one server should
 * not simply move to the next. Each controller is its own audited, revertible ban; one that has no such
 * account, or is already banned there, is reported rather than failing the rest.
 */
export async function banEverywhere(
  me: Staff,
  username: string,
  reason: string,
): Promise<FleetOutcome[]> {
  reasonOf(reason);
  const all = fleet();
  const reachable = (c: (typeof all)[number]) => c.state === "online" || c.state === "stale";
  const targets = all.filter(reachable);
  const skipped = all.filter((c) => !reachable(c));
  const results = await Promise.all(
    targets.map(async (c): Promise<FleetOutcome> => {
      try {
        await banPlayer(me, c.id, username, reason);
        return { controllerId: c.id, name: c.name, result: "done" };
      } catch (error) {
        const code = error instanceof DomainError ? error.code : "unknown";
        if (code === "not_found") return { controllerId: c.id, name: c.name, result: "absent" };
        return { controllerId: c.id, name: c.name, result: "failed", code };
      }
    }),
  );
  return [
    ...results,
    ...skipped.map((c) => ({ controllerId: c.id, name: c.name, result: "skipped" as const })),
  ];
}

/** The backend's own access state: approval for invite-only servers, and disabling an account. */
export async function setPlayerAccess(
  me: Staff,
  controllerId: string,
  userId: string,
  change: { status?: BackendAccount["status"]; approvalStatus?: BackendAccount["approvalStatus"] },
  reason: string,
  revertOfId?: number,
) {
  const api = backend(controllerId);
  const [beforeAccount] = (await api.playersById([userId], op(me))).accounts;
  if (!beforeAccount) throw new DomainError("not_found");
  const { account } = await api.setAccess(userId, { ...change, reason: reasonOf(reason) }, op(me));
  recordAudit(me, {
    action: "player.access",
    targetType: "player",
    targetId: userId,
    targetLabel: account.displayName,
    controllerId,
    reason,
    before: { status: beforeAccount.status, approvalStatus: beforeAccount.approvalStatus },
    after: { status: account.status, approvalStatus: account.approvalStatus },
    revertible: true,
    revertOfId,
  });
  return account;
}

export async function setPlayerRoles(
  me: Staff,
  controllerId: string,
  userId: string,
  roles: string[],
  reason: string,
  revertOfId?: number,
) {
  const api = backend(controllerId);
  const [beforeAccount] = (await api.playersById([userId], op(me))).accounts;
  if (!beforeAccount) throw new DomainError("not_found");
  const { account } = await api.setRoles(userId, roles, reasonOf(reason), op(me));
  recordAudit(me, {
    action: "player.roles",
    targetType: "player",
    targetId: userId,
    targetLabel: account.displayName,
    controllerId,
    reason,
    before: { roles: beforeAccount.roles },
    after: { roles: account.roles },
    revertible: true,
    revertOfId,
  });
  return account;
}
