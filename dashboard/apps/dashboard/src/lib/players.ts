import { recordAudit } from "@/lib/audit/record";
import type { Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { adminApi, type GamePlayer } from "@/lib/game/admin-api";
import { type BackendAccount, backend } from "@/lib/game/backend";

/**
 * Moderation as the game enforces it. The game answers first; the audit event is written after, with
 * the player's restriction before and after, which is what a revert puts back.
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

async function current(username: string, me: Staff): Promise<GamePlayer> {
  const p = await adminApi.player(username, me.discordId);
  if (!p) throw new DomainError("not_found");
  return p;
}

export async function kickPlayer(me: Staff, username: string, reason: string | null) {
  const before = await current(username, me);
  const result = await adminApi.kick(before.username, me.discordId);
  recordAudit(me, {
    action: "player.kick",
    targetType: "player",
    targetId: before.userId,
    targetLabel: before.username,
    reason,
    after: { connectionsClosed: result.connectionsClosed },
  });
  return result;
}

export async function timeoutPlayer(
  me: Staff,
  username: string,
  until: { minutes?: number; until?: string },
  reason: string,
  revertOfId?: number,
) {
  const before = await current(username, me);
  const after = await adminApi.timeout(before.username, until, reasonOf(reason), me.discordId);
  recordAudit(me, {
    action: "player.timeout",
    targetType: "player",
    targetId: after.userId,
    targetLabel: after.username,
    reason,
    before: restrictionOf(before),
    after: restrictionOf(after),
    revertible: true,
    revertOfId,
  });
  return after;
}

export async function banPlayer(me: Staff, username: string, reason: string, revertOfId?: number) {
  const before = await current(username, me);
  const after = await adminApi.ban(before.username, reasonOf(reason), me.discordId);
  recordAudit(me, {
    action: "player.ban",
    targetType: "player",
    targetId: after.userId,
    targetLabel: after.username,
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
  username: string,
  reason: string | null,
  revertOfId?: number,
) {
  const before = await current(username, me);
  const after = await adminApi.unban(before.username, reason?.trim() || null, me.discordId);
  recordAudit(me, {
    action: "player.unban",
    targetType: "player",
    targetId: after.userId,
    targetLabel: after.username,
    reason,
    before: restrictionOf(before),
    after: restrictionOf(after),
    revertible: true,
    revertOfId,
  });
  return after;
}

export async function liftTimeout(me: Staff, username: string, revertOfId?: number) {
  const before = await current(username, me);
  const after = await adminApi.liftTimeout(before.username, me.discordId);
  recordAudit(me, {
    action: "player.lift_timeout",
    targetType: "player",
    targetId: after.userId,
    targetLabel: after.username,
    before: restrictionOf(before),
    after: restrictionOf(after),
    revertible: true,
    revertOfId,
  });
  return after;
}

/** The backend's own access state: approval for invite-only servers, and disabling an account. */
export async function setPlayerAccess(
  me: Staff,
  userId: string,
  change: { status?: BackendAccount["status"]; approvalStatus?: BackendAccount["approvalStatus"] },
  reason: string,
  revertOfId?: number,
) {
  const [beforeAccount] = (await backend.playersById([userId], op(me))).accounts;
  if (!beforeAccount) throw new DomainError("not_found");
  const { account } = await backend.setAccess(
    userId,
    { ...change, reason: reasonOf(reason) },
    op(me),
  );
  recordAudit(me, {
    action: "player.access",
    targetType: "player",
    targetId: userId,
    targetLabel: account.displayName,
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
  userId: string,
  roles: string[],
  reason: string,
  revertOfId?: number,
) {
  const [beforeAccount] = (await backend.playersById([userId], op(me))).accounts;
  if (!beforeAccount) throw new DomainError("not_found");
  const { account } = await backend.setRoles(userId, roles, reasonOf(reason), op(me));
  recordAudit(me, {
    action: "player.roles",
    targetType: "player",
    targetId: userId,
    targetLabel: account.displayName,
    reason,
    before: { roles: beforeAccount.roles },
    after: { roles: account.roles },
    revertible: true,
    revertOfId,
  });
  return account;
}
