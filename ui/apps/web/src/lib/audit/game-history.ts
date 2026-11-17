import type { AuditEvent } from "@/lib/db/schema";
import type { BackendAuditRecord } from "@/lib/game/backend";

/** Who the game's record names, made readable: a key id to its name, a Discord id to the staff member. */
export interface ActorNames {
  keys: Map<string, string>;
  staff: Map<string, string>;
}

// A dashboard row and the game's record of the same call land within seconds of each other
const SAME_CALL_MS = 60_000;

/**
 * `key:<id>[/discord:<id>]`, `host`, `system`, a manager operator: the name History shows. Never a key: the
 * record holds only ids.
 */
export function actorName(actor: string, names: ActorNames): string {
  const [who, discord] = actor.split("/discord:");
  if (discord) return names.staff.get(discord) ?? `discord:${discord}`;
  if (who?.startsWith("key:")) {
    const id = who.slice(4);
    return names.keys.get(id) ?? who;
  }
  return who || "game";
}

/**
 * The game server's own admin record for one player, as History rows: its admin sign-in, the admin API with
 * any key (console, scripts, the Discord bot), the host's tools and remote controllers all write there. A
 * call this dashboard made is already a row of its own: one signed `manager:<id>`, or the same action on the
 * same player within a minute of one, is left out. Rows get negative ids: read-only, no Undo.
 */
export function gameHistory(
  records: BackendAuditRecord[],
  userId: string,
  name: string,
  controllerId: string,
  own: AuditEvent[] = [],
  names: ActorNames = { keys: new Map(), staff: new Map() },
): AuditEvent[] {
  return records
    .filter((r) => r.targetUserId === userId && !r.actorUserId?.startsWith("manager:"))
    .map((r, i) => {
      const at = Date.parse(r.createdAt ?? "") || 0;
      const actor = r.actorUserId ?? "game";
      const state = r.newState && typeof r.newState === "object" ? r.newState : {};
      return {
        id: -(i + 1),
        at,
        actorId: null,
        actorName: actorName(actor, names),
        controllerId,
        action: r.action,
        targetType: "player",
        targetId: userId,
        targetLabel: name,
        reason: r.reason ?? null,
        before: r.oldState ?? null,
        after: { ...state, actor },
        revertible: false,
        revertOfId: null,
        revertedById: null,
        ip: null,
      };
    })
    .filter(
      (g) =>
        !own.some(
          (o) =>
            o.action === g.action &&
            o.targetId === g.targetId &&
            Math.abs(o.at - g.at) < SAME_CALL_MS,
        ),
    );
}
