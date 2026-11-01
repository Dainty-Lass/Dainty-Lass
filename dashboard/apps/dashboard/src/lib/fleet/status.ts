import { type AgentStatus, collectStatus, HEARTBEAT_MS, STALE_MS } from "@dainty/fleet";
import { and, eq, gte, lt } from "drizzle-orm";
import { db, schema } from "@/lib/db";
import type { Controller } from "@/lib/db/schema";
import { ensureLocal, LOCAL, listControllers } from "./controllers";
import { type Performance, performance, recordMetrics } from "./metrics";
import { connectedSince, isConnected } from "./registry";
import { localGameConfig } from "./transport";

/**
 * Status is pushed, not pulled: each agent reports every HEARTBEAT_MS, and the dashboard reports its own
 * controller on the same clock. Pages read the last report, so a fleet of hundreds costs a page view
 * one query instead of hundreds of calls.
 */

export type ControllerState =
  | "online"
  | "stale"
  | "offline"
  | "pending"
  | "disabled"
  | "not_configured";

export interface ControllerView {
  id: string;
  kind: Controller["kind"];
  name: string;
  region: string | null;
  note: string | null;
  version: string | null;
  state: ControllerState;
  lastSeenAt: number | null;
  connectedSince: number | null;
  pairingExpiresAt: number | null;
  status: AgentStatus | null;
}

export function recordStatus(controllerId: string, status: AgentStatus): void {
  const now = Date.now();
  db.update(schema.controllers)
    .set({ status, lastSeenAt: now, version: status.version.slice(0, 32) })
    .where(eq(schema.controllers.id, controllerId))
    .run();
  // One sample a minute is enough for a week-long chart, whatever the heartbeat.
  const minute = Math.floor(now / 60_000) * 60_000;
  db.insert(schema.controllerSamples)
    .values({
      controllerId,
      at: minute,
      online: status.online ?? status.server?.onlineCount ?? 0,
      huntsRunning: status.hunts?.running ?? 0,
      huntsMax: status.hunts?.maxHuntServers ?? 0,
    })
    .onConflictDoNothing()
    .run();
  if (status.metrics) recordMetrics(controllerId, status.metrics, now);
}

function stateOf(c: Controller, status: AgentStatus | null): ControllerState {
  if (c.disabled) return "disabled";
  if (c.kind === "agent" && !c.tokenHash) return "pending";
  if (c.kind === "local" && status?.adminApi === "not_configured") return "not_configured";
  if (!c.lastSeenAt) return "offline";
  if (c.kind === "agent" && !isConnected(c.id)) return "offline";
  return Date.now() - c.lastSeenAt > STALE_MS ? "stale" : "online";
}

export function viewOf(c: Controller): ControllerView {
  const stored = (c.status as AgentStatus | null) ?? null;
  // The metrics summary is for the performance section (lib/fleet/metrics.ts), not every list of controllers.
  const status = stored ? { ...stored, metrics: undefined } : null;
  return {
    id: c.id,
    kind: c.kind,
    name: c.name,
    region: c.region,
    note: c.note,
    version: c.version,
    state: stateOf(c, status),
    lastSeenAt: c.lastSeenAt,
    connectedSince: c.kind === "local" ? null : connectedSince(c.id),
    pairingExpiresAt: c.pairingExpiresAt,
    status,
  };
}

export function fleet(): ControllerView[] {
  startLocalReporter();
  return listControllers().map(viewOf);
}

/** The performance section for these controllers (all of them without ids), from what they last reported. */
export function fleetPerformance(controllerIds: string[] | undefined, chat: boolean): Performance {
  startLocalReporter();
  const rows = listControllers().filter(
    (c) =>
      (!controllerIds || controllerIds.includes(c.id)) && !(c.kind === "agent" && !c.tokenHash),
  );
  return performance(
    rows.map((c) => ({ id: c.id, name: c.name, state: viewOf(c).state, controller: c })),
    { chat },
  );
}

export interface FleetTotals {
  controllers: number;
  online: number;
  players: number;
  huntsRunning: number;
  huntsMax: number;
}

export function totals(views: ControllerView[]): FleetTotals {
  const live = views.filter((v) => v.state === "online" || v.state === "stale");
  return {
    controllers: views.length,
    online: live.length,
    players: live.reduce((n, v) => n + (v.status?.online ?? v.status?.server?.onlineCount ?? 0), 0),
    huntsRunning: live.reduce((n, v) => n + (v.status?.hunts?.running ?? 0), 0),
    huntsMax: live.reduce((n, v) => n + (v.status?.hunts?.maxHuntServers ?? 0), 0),
  };
}

/**
 * Fleet-wide players and hunts over the last `hours`, in 10-minute buckets, oldest first. Each
 * controller's peak in a bucket is summed, and empty buckets are kept so the time axis stays even.
 */
export function history(
  controllerIds: string[],
  hours: number,
): { at: number; online: number; hunts: number }[] {
  const BUCKET = 600_000;
  const start = Math.floor((Date.now() - hours * 3_600_000) / BUCKET) * BUCKET;
  const ids = new Set(controllerIds);
  const peaks = new Map<string, { online: number; hunts: number }>();
  for (const r of db
    .select()
    .from(schema.controllerSamples)
    .where(gte(schema.controllerSamples.at, start))
    .all()) {
    if (!ids.has(r.controllerId)) continue;
    const key = `${Math.floor(r.at / BUCKET) * BUCKET}|${r.controllerId}`;
    const cur = peaks.get(key) ?? { online: 0, hunts: 0 };
    peaks.set(key, {
      online: Math.max(cur.online, r.online),
      hunts: Math.max(cur.hunts, r.huntsRunning),
    });
  }
  const sums = new Map<number, { online: number; hunts: number }>();
  for (const [key, v] of peaks) {
    const at = Number(key.split("|")[0]);
    const cur = sums.get(at) ?? { online: 0, hunts: 0 };
    sums.set(at, { online: cur.online + v.online, hunts: cur.hunts + v.hunts });
  }
  const count = Math.ceil((Date.now() - start) / BUCKET);
  return Array.from({ length: count }, (_, i) => {
    const at = start + i * BUCKET;
    return { at, ...(sums.get(at) ?? { online: 0, hunts: 0 }) };
  });
}

export function pruneSamples(): void {
  db.delete(schema.controllerSamples)
    .where(lt(schema.controllerSamples.at, Date.now() - 7 * 86_400_000))
    .run();
}

export function collectLocalStatus(): Promise<AgentStatus> {
  return collectStatus(localGameConfig(), process.env.NEXT_PUBLIC_APP_VERSION ?? "dev");
}

/** The dashboard's own controller reports on the agents' clock, from inside this process. */
export function startLocalReporter(): void {
  const g = globalThis as unknown as { __daintyLocalReporter?: ReturnType<typeof setInterval> };
  if (g.__daintyLocalReporter || process.env.DASHBOARD_DATABASE_FILE === ":memory:") return;
  ensureLocal();
  const tick = async () => {
    try {
      recordStatus(LOCAL, await collectLocalStatus());
      pruneSamples();
    } catch (error) {
      console.error("[fleet] local status failed:", error instanceof Error ? error.message : error);
    }
  };
  void tick();
  g.__daintyLocalReporter = setInterval(tick, HEARTBEAT_MS);
}

export function recentSamples(controllerId: string, since: number) {
  return db
    .select()
    .from(schema.controllerSamples)
    .where(
      and(
        eq(schema.controllerSamples.controllerId, controllerId),
        gte(schema.controllerSamples.at, since),
      ),
    )
    .orderBy(schema.controllerSamples.at)
    .all();
}
