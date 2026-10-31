import { config } from "@/lib/config";
import { DomainError, type ErrorCode } from "@/lib/errors";

/**
 * The admin service's GraphQL (dainty-lass docs/admin-api.md): live status and the moderation the
 * game enforces. Loopback only, so the dashboard runs on the server or reaches it through a tunnel.
 */

export type AccountStatus = "ACTIVE" | "BANNED" | "DISABLED" | "UNKNOWN";
export type RestrictionKind = "BAN" | "TIMEOUT" | "DISABLED";

export interface Restriction {
  kind: RestrictionKind;
  reason: string | null;
  since: string | null;
  by: string | null;
  until: string | null;
}

export interface GamePlayer {
  userId: string;
  username: string;
  status: AccountStatus;
  restriction: Restriction | null;
  online: boolean;
  location: {
    kind: "HUB" | "HUNT" | "TRAINING" | "TUTORIAL";
    instanceId: string;
    title: string;
  } | null;
  admin: boolean;
  createdAt: string | null;
  lastLoginAt: string | null;
}

export interface ServerStatus {
  name: string;
  mode: "PRIVATE" | "PUBLIC";
  registration: string;
  version: string;
  commit: string | null;
  uptimeSeconds: number;
  onlineCount: number;
  chat: boolean;
  gameData: string;
}

export interface HuntStatus {
  maxHuntServers: number;
  running: number;
  free: number;
  hunts: {
    id: string;
    title: string;
    map: string;
    behemoth: string | null;
    playerCount: number;
    maxPlayers: number;
    players: { userId: string; username: string | null }[];
    startedAt: string | null;
  }[];
}

const PLAYER = `userId username status online admin createdAt lastLoginAt
  restriction { kind reason since by until } location { kind instanceId title }`;

// The admin API's codes, mapped onto the catalog's; anything else is the game refusing.
const CODES: Record<string, ErrorCode> = {
  BAD_INPUT: "invalid_input",
  NOT_FOUND: "not_found",
  PROTECTED_ACCOUNT: "protected_account",
  ALREADY_BANNED: "already_banned",
  NOT_BANNED: "not_banned",
  NOT_TIMED_OUT: "not_timed_out",
  BACKEND_UNAVAILABLE: "game_unavailable",
};

export function adminApiConfigured(): boolean {
  return Boolean(config.adminApi.key);
}

async function graphql<T>(
  query: string,
  variables: Record<string, unknown>,
  operatorId: string,
): Promise<T> {
  if (!config.adminApi.key) throw new DomainError("game_not_configured", "DASHBOARD_ADMIN_API_KEY");
  let response: Response;
  try {
    response = await fetch(config.adminApi.url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${config.adminApi.key}`,
        // Digits only; the backend's audit records it as manager:<id>.
        "x-reforged-operator-id": /^\d{1,32}$/.test(operatorId) ? operatorId : "0",
      },
      body: JSON.stringify({ query, variables }),
      signal: AbortSignal.timeout(15_000),
    });
  } catch {
    throw new DomainError("game_unavailable");
  }
  const body = (await response.json().catch(() => null)) as {
    data?: T;
    errors?: { message: string; extensions?: { code?: string } }[];
  } | null;
  const error = body?.errors?.[0];
  if (error)
    throw new DomainError(CODES[error.extensions?.code ?? ""] ?? "game_refused", error.message);
  if (!response.ok || !body?.data) throw new DomainError("game_refused", `HTTP ${response.status}`);
  return body.data;
}

export const adminApi = {
  serverStatus: (op = "0") =>
    graphql<{ serverStatus: ServerStatus }>(
      "{ serverStatus { name mode registration version commit uptimeSeconds onlineCount chat gameData } }",
      {},
      op,
    ).then((d) => d.serverStatus),
  huntStatus: (op = "0") =>
    graphql<{ huntStatus: HuntStatus }>(
      "{ huntStatus { maxHuntServers running free hunts { id title map behemoth playerCount maxPlayers startedAt players { userId username } } } }",
      {},
      op,
    ).then((d) => d.huntStatus),
  player: (username: string, op = "0") =>
    graphql<{ player: GamePlayer | null }>(
      `query($u: String!) { player(username: $u) { ${PLAYER} } }`,
      { u: username },
      op,
    ).then((d) => d.player),
  kick: (username: string, op: string) =>
    graphql<{ kick: { userId: string; username: string; connectionsClosed: number } }>(
      "mutation($u: String!) { kick(username: $u) { userId username connectionsClosed } }",
      { u: username },
      op,
    ).then((d) => d.kick),
  timeout: (
    username: string,
    args: { minutes?: number; until?: string },
    reason: string,
    op: string,
  ) =>
    graphql<{ timeout: GamePlayer }>(
      `mutation($u: String!, $m: Int, $t: String, $r: String!) { timeout(username: $u, minutes: $m, until: $t, reason: $r) { ${PLAYER} } }`,
      { u: username, m: args.minutes ?? null, t: args.until ?? null, r: reason },
      op,
    ).then((d) => d.timeout),
  liftTimeout: (username: string, op: string) =>
    graphql<{ liftTimeout: GamePlayer }>(
      `mutation($u: String!) { liftTimeout(username: $u) { ${PLAYER} } }`,
      { u: username },
      op,
    ).then((d) => d.liftTimeout),
  ban: (username: string, reason: string, op: string) =>
    graphql<{ ban: GamePlayer }>(
      `mutation($u: String!, $r: String!) { ban(username: $u, reason: $r) { ${PLAYER} } }`,
      { u: username, r: reason },
      op,
    ).then((d) => d.ban),
  unban: (username: string, reason: string | null, op: string) =>
    graphql<{ unban: GamePlayer }>(
      `mutation($u: String!, $r: String) { unban(username: $u, reason: $r) { ${PLAYER} } }`,
      { u: username, r: reason },
      op,
    ).then((d) => d.unban),
};
