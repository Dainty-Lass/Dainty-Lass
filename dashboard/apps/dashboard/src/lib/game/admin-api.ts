import type { GamePlayer, HuntStatus, ServerStatus } from "@dainty/fleet";
import { config } from "@/lib/config";
import { game } from "@/lib/fleet/transport";

/**
 * The admin service's GraphQL (dainty-lass docs/admin-api.md) on one controller: live status and the
 * moderation the game enforces. The documents themselves live in @dainty/fleet, which agents allowlist.
 */

export type {
  AccountStatus,
  GamePlayer,
  HuntStatus,
  Restriction,
  RestrictionKind,
  ServerStatus,
} from "@dainty/fleet";

export function adminApiConfigured(): boolean {
  return Boolean(config.adminApi.key);
}

export function adminApi(controllerId: string) {
  const g = game(controllerId);
  return {
    serverStatus: (op = "0") => g.admin<ServerStatus>("serverStatus", {}, op),
    huntStatus: (op = "0") => g.admin<HuntStatus>("huntStatus", {}, op),
    player: (username: string, op = "0") =>
      g.admin<GamePlayer | null>("player", { u: username }, op),
    kick: (username: string, op: string) =>
      g.admin<{ userId: string; username: string; connectionsClosed: number }>(
        "kick",
        { u: username },
        op,
      ),
    timeout: (
      username: string,
      args: { minutes?: number; until?: string },
      reason: string,
      op: string,
    ) =>
      g.admin<GamePlayer>(
        "timeout",
        { u: username, m: args.minutes ?? null, t: args.until ?? null, r: reason },
        op,
      ),
    liftTimeout: (username: string, op: string) =>
      g.admin<GamePlayer>("liftTimeout", { u: username }, op),
    ban: (username: string, reason: string, op: string) =>
      g.admin<GamePlayer>("ban", { u: username, r: reason }, op),
    unban: (username: string, reason: string | null, op: string) =>
      g.admin<GamePlayer>("unban", { u: username, r: reason }, op),
  };
}
