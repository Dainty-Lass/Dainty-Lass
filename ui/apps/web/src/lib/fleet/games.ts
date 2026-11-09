import { type GameConfig, runAdminQuery } from "@dainty/fleet";

/**
 * The game modules and the fleet, as the admin API on this host answers them (module-sharing plan A4, A6):
 * the game servers and availability of the dashboard's own server, its kept events, and every install a
 * remote controller manages (D1). Read on the reporter's clock and kept in memory, so pages never call out.
 * Local only: these are not agent operations, and the agent gains no game-server fields.
 */

export type GameServerKind = "HUB" | "HUNT" | "DOJO" | "TUTORIAL";

export interface GameServer {
  game: string;
  id: string;
  kind: GameServerKind;
  title: string;
  map: string;
  host: string;
  port: number;
  players: number;
  maxPlayers: number;
  ready: boolean;
  startedAt: string | null;
}

export interface Availability {
  game: string;
  up: boolean;
  /** A code: director_unreachable, no_hub, hub_not_ready or module_not_ready. */
  reason: string | null;
  hub: GameServer | null;
  freeHunts: number | null;
}

export interface GameEvent {
  seq: number;
  atMs: number;
  name: string;
  source: string | null;
}

export interface Install {
  node: string;
  region: string;
  label: string;
  state: "online" | "degraded" | "offline" | "revoked";
  lastReportAt: string | null;
  version: string | null;
  playersOnline: number | null;
  availability: Availability | null;
  gameServers: GameServer[];
}

export interface GamesReport {
  at: number;
  /** Null when the admin API could not answer. */
  gameServers: GameServer[] | null;
  availability: Availability | null;
  /** Newest first, at most KEPT_EVENTS. */
  events: GameEvent[];
  /** Null when this server is not a remote controller (NO_FLEET) or could not answer. */
  installs: Install[] | null;
}

const SERVER = "game id kind title map host port players maxPlayers ready startedAt";
const AVAILABILITY = `game up reason freeHunts hub { ${SERVER} }`;
const Q = {
  gameServers: `{ gameServers { ${SERVER} } }`,
  availability: `{ availability { ${AVAILABILITY} } }`,
  events: "query($a: Int) { events(after: $a, limit: 1000) { seq atMs name source } }",
  installs: `{ installs { node region label state lastReportAt version playersOnline availability { ${AVAILABILITY} } gameServers { ${SERVER} } } }`,
};

export const KEPT_EVENTS = 50;
// Bounds the first read after a restart, which starts from the oldest kept row.
const EVENT_PAGES = 20;

const g = globalThis as unknown as {
  __daintyGames?: { report: GamesReport | null; cursor: number; events: GameEvent[] };
};
const memory = () => {
  g.__daintyGames ??= { report: null, cursor: 0, events: [] };
  return g.__daintyGames;
};

async function newEvents(
  cfg: GameConfig,
  from: number,
): Promise<{ cursor: number; rows: GameEvent[] }> {
  let cursor = from;
  const rows: GameEvent[] = [];
  for (let page = 0; page < EVENT_PAGES; page++) {
    const r = await runAdminQuery<GameEvent[]>(cfg, "events", Q.events, { a: cursor }, "0");
    if (!r.ok || r.data.length === 0) break;
    rows.push(...r.data);
    cursor = r.data[r.data.length - 1]!.seq;
    if (r.data.length < 1000) break;
  }
  return { cursor, rows };
}

/** One read of everything above; the reporter calls it every heartbeat. */
export async function refreshGames(cfg: GameConfig): Promise<GamesReport> {
  const m = memory();
  const [servers, availability, installs, events] = await Promise.all([
    runAdminQuery<GameServer[]>(cfg, "gameServers", Q.gameServers, {}, "0"),
    runAdminQuery<Availability>(cfg, "availability", Q.availability, {}, "0"),
    runAdminQuery<Install[]>(cfg, "installs", Q.installs, {}, "0"),
    newEvents(cfg, m.cursor),
  ]);
  m.cursor = events.cursor;
  m.events = [...events.rows.reverse(), ...m.events].slice(0, KEPT_EVENTS);
  m.report = {
    at: Date.now(),
    gameServers: servers.ok ? servers.data : null,
    availability: availability.ok ? availability.data : null,
    events: m.events,
    installs: installs.ok ? installs.data : null,
  };
  return m.report;
}

/** The last report, or null before the first. */
export function gamesReport(): GamesReport | null {
  return memory().report;
}

/** Forgets every report; for tests. */
export function resetGames(): void {
  g.__daintyGames = undefined;
}
