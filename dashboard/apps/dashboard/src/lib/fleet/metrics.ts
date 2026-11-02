import {
  type AgentStatus,
  histogramQuantile,
  type MetricsSummary,
  type RouteMetrics,
} from "@dainty/fleet";
import type { Controller } from "@/lib/db/schema";
import type { ControllerState } from "./status";

/**
 * An hour of each controller's metrics, in memory: the counters as they were at every heartbeat, so
 * rates and the hour's charts come from differences between samples. The dashboard is one process
 * (AGENTS.md), so this is the whole history; a restart starts the hour again, and the last summary
 * itself survives in the controller's stored status.
 */

export const RING_MS = 3_600_000;
/** Rates and latency are over this much of the ring. */
export const RATE_WINDOW_MS = 5 * 60_000;
const MINUTE = 60_000;

interface RoutePoint {
  requests: number;
  errors: number;
  buckets: number[];
  count: number;
}

interface Point {
  at: number;
  routes: Record<string, RoutePoint>;
  chat: Record<string, number>;
  cpu: number;
  memory: number;
}

type Ring = Map<string, Point[]>;
// Kept on globalThis so a dev reload keeps the hour.
const holder = globalThis as unknown as { __daintyMetrics?: Ring };
holder.__daintyMetrics ??= new Map();
const rings: Ring = holder.__daintyMetrics;

const routeKey = (r: { service: string; route: string }) => `${r.service}\u0000${r.route}`;

export function recordMetrics(
  controllerId: string,
  summary: MetricsSummary,
  at = Date.now(),
): void {
  const routes: Record<string, RoutePoint> = {};
  for (const r of summary.http)
    routes[routeKey(r)] = {
      requests: r.requests,
      errors: r.errors,
      buckets: r.buckets,
      count: r.count,
    };
  const ring = rings.get(controllerId) ?? [];
  ring.push({
    at,
    routes,
    chat: { ...summary.chat },
    cpu: summary.processes.reduce((n, p) => n + p.cpuPercent, 0),
    memory: summary.processes.reduce((n, p) => n + p.memoryBytes, 0),
  });
  while (ring.length && ring[0]!.at < at - RING_MS) ring.shift();
  rings.set(controllerId, ring);
}

export function forgetMetrics(controllerId?: string): void {
  if (controllerId) rings.delete(controllerId);
  else rings.clear();
}

/** A counter's growth from a to b; a drop is a restart, after which b is all new. */
const grew = (a: number | undefined, b: number) => (a === undefined || b < a ? b : b - a);

function routeIncrease(points: Point[], key: string): RoutePoint | null {
  let total: RoutePoint | null = null;
  for (let i = 1; i < points.length; i++) {
    const b = points[i]!.routes[key];
    if (!b) continue;
    const a = points[i - 1]!.routes[key];
    // A restart resets every series at once; the request count is the one to judge it by.
    const reset = !a || b.count < a.count || b.requests < a.requests;
    const step: RoutePoint = {
      requests: reset ? b.requests : b.requests - a.requests,
      errors: reset ? b.errors : grew(a.errors, b.errors),
      count: reset ? b.count : b.count - a.count,
      buckets: b.buckets.map((n, j) => (reset || !a ? n : grew(a.buckets[j], n))),
    };
    if (!total) total = step;
    else {
      total.requests += step.requests;
      total.errors += step.errors;
      total.count += step.count;
      total.buckets =
        total.buckets.length === step.buckets.length
          ? total.buckets.map((n, j) => n + step.buckets[j]!)
          : step.buckets;
    }
  }
  return total;
}

export interface RouteRate {
  service: string;
  route: string;
  /** Requests a second over the window; null until two samples are in it. */
  perSecond: number | null;
  /** The share of answers that were 5xx. */
  errorShare: number | null;
  p50: number | null;
  p95: number | null;
}

interface Window {
  points: Point[];
  seconds: number;
}

function windowOf(ring: Point[], now: number, ms: number): Window {
  const points = ring.filter((p) => p.at >= now - ms);
  const seconds = points.length > 1 ? (points.at(-1)!.at - points[0]!.at) / 1000 : 0;
  return { points, seconds };
}

function rateOf(
  route: { service: string; route: string; bounds: number[] },
  inc: RoutePoint | null,
  seconds: number,
): RouteRate {
  const base = { service: route.service, route: route.route };
  if (!inc || seconds <= 0)
    return { ...base, perSecond: null, errorShare: null, p50: null, p95: null };
  return {
    ...base,
    perSecond: inc.requests / seconds,
    errorShare: inc.requests > 0 ? inc.errors / inc.requests : null,
    p50: histogramQuantile(0.5, route.bounds, inc.buckets, inc.count),
    p95: histogramQuantile(0.95, route.bounds, inc.buckets, inc.count),
  };
}

export interface SeriesPoint {
  at: number;
  requests: number;
  errors: number;
  cpu: number;
  memory: number;
}

/** The hour as one point a minute, oldest first; minutes with no sample are zero. */
function series(ring: Point[], now: number): SeriesPoint[] {
  const start = Math.floor(now / MINUTE) * MINUTE - (60 - 1) * MINUTE;
  const out: SeriesPoint[] = Array.from({ length: 60 }, (_, i) => ({
    at: start + i * MINUTE,
    requests: 0,
    errors: 0,
    cpu: 0,
    memory: 0,
  }));
  const seen = new Array<number>(60).fill(0);
  for (let i = 0; i < ring.length; i++) {
    const b = ring[i]!;
    const slot = Math.floor((b.at - start) / MINUTE);
    if (slot < 0 || slot >= 60) continue;
    const o = out[slot]!;
    o.cpu += b.cpu;
    o.memory += b.memory;
    seen[slot]!++;
    const a = ring[i - 1];
    if (!a) continue;
    for (const key of Object.keys(b.routes)) {
      const inc = routeIncrease([a, b], key);
      if (!inc) continue;
      o.requests += inc.requests;
      o.errors += inc.errors;
    }
  }
  for (let i = 0; i < 60; i++) {
    const n = seen[i]!;
    if (n > 1) {
      out[i]!.cpu /= n;
      out[i]!.memory /= n;
    }
  }
  return out;
}

function chatIncrease(points: Point[]): Record<string, number> {
  const out: Record<string, number> = {};
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1]!.chat;
    const b = points[i]!.chat;
    // A restart zeroes every verdict together: judge by the total.
    const sum = (c: Record<string, number>) => Object.values(c).reduce((n, v) => n + v, 0);
    const reset = sum(b) < sum(a);
    for (const [verdict, n] of Object.entries(b))
      out[verdict] = (out[verdict] ?? 0) + (reset ? n : grew(a[verdict] ?? 0, n));
  }
  return out;
}

export interface ControllerPerformance {
  id: string;
  name: string;
  state: ControllerState;
  /** The last summary, or null when this controller has never reported one. */
  summary: MetricsSummary | null;
  routes: RouteRate[];
  /** Every route's latency together. */
  overall: RouteRate | null;
  chat: Record<string, number> | null;
  series: SeriesPoint[];
}

export interface Performance {
  controllers: ControllerPerformance[];
  /** Each service and route summed over the controllers. */
  routes: RouteRate[];
  chat: Record<string, number> | null;
  /** How far back the chat counts reach: the ring may be younger than an hour. */
  chatSinceMs: number | null;
  series: SeriesPoint[];
}

function mergeRoutes(
  parts: { route: RouteMetrics; inc: RoutePoint | null; seconds: number }[],
  service: string,
  route: string,
): RouteRate {
  const usable = parts.filter((p) => p.inc && p.seconds > 0);
  const bounds = usable[0]?.route.bounds ?? [];
  const same = usable.filter((p) => p.route.bounds.join() === bounds.join());
  if (!usable.length)
    return { service, route, perSecond: null, errorShare: null, p50: null, p95: null };
  const perSecond = usable.reduce((n, p) => n + p.inc!.requests / p.seconds, 0);
  const requests = usable.reduce((n, p) => n + p.inc!.requests, 0);
  const errors = usable.reduce((n, p) => n + p.inc!.errors, 0);
  const count = same.reduce((n, p) => n + p.inc!.count, 0);
  const buckets = bounds.map((_, j) => same.reduce((n, p) => n + (p.inc!.buckets[j] ?? 0), 0));
  return {
    service,
    route,
    perSecond,
    errorShare: requests > 0 ? errors / requests : null,
    p50: histogramQuantile(0.5, bounds, buckets, count),
    p95: histogramQuantile(0.95, bounds, buckets, count),
  };
}

export interface PerformanceInput {
  id: string;
  name: string;
  state: ControllerState;
  controller: Pick<Controller, "status">;
}

/**
 * Rates, latency and the hour's series for each controller and summed over them. `chat` is false where
 * the reader may not see chat: the counts are then never computed, let alone sent.
 */
export function performance(
  inputs: PerformanceInput[],
  { chat, now = Date.now() }: { chat: boolean; now?: number },
): Performance {
  const fleetRoutes = new Map<
    string,
    { service: string; route: string; parts: Parameters<typeof mergeRoutes>[0] }
  >();
  let chatSince: number | null = null;
  const controllers = inputs.map((input): ControllerPerformance => {
    const summary = ((input.controller.status as AgentStatus | null)?.metrics ??
      null) as MetricsSummary | null;
    const ring = rings.get(input.id) ?? [];
    const win = windowOf(ring, now, RATE_WINDOW_MS);
    const routes: RouteRate[] = [];
    const all: RoutePoint = { requests: 0, errors: 0, count: 0, buckets: [] };
    let allBounds: number[] | null = null;
    for (const r of summary?.http ?? []) {
      const inc = routeIncrease(win.points, routeKey(r));
      routes.push(rateOf(r, inc, win.seconds));
      const key = routeKey(r);
      const entry = fleetRoutes.get(key) ?? { service: r.service, route: r.route, parts: [] };
      entry.parts.push({ route: r, inc, seconds: win.seconds });
      fleetRoutes.set(key, entry);
      if (!inc) continue;
      all.requests += inc.requests;
      all.errors += inc.errors;
      if (allBounds === null) allBounds = r.bounds;
      if (allBounds.join() === r.bounds.join()) {
        all.count += inc.count;
        all.buckets = allBounds.map((_, j) => (all.buckets[j] ?? 0) + (inc.buckets[j] ?? 0));
      }
    }
    if (chat && ring.length > 1) chatSince = Math.min(chatSince ?? now, ring[0]!.at);
    return {
      id: input.id,
      name: input.name,
      state: input.state,
      summary,
      routes,
      overall:
        summary && win.seconds > 0
          ? rateOf({ service: "", route: "", bounds: allBounds ?? [] }, all, win.seconds)
          : null,
      chat: chat ? chatIncrease(ring) : null,
      series: series(ring, now),
    };
  });

  const sumSeries = controllers[0]?.series.map((p, i) => ({
    at: p.at,
    requests: controllers.reduce((n, c) => n + c.series[i]!.requests, 0),
    errors: controllers.reduce((n, c) => n + c.series[i]!.errors, 0),
    cpu: controllers.reduce((n, c) => n + c.series[i]!.cpu, 0),
    memory: controllers.reduce((n, c) => n + c.series[i]!.memory, 0),
  }));
  let fleetChat: Record<string, number> | null = null;
  if (chat) {
    fleetChat = {};
    for (const c of controllers)
      for (const [verdict, n] of Object.entries(c.chat ?? {}))
        fleetChat[verdict] = (fleetChat[verdict] ?? 0) + n;
  }
  return {
    controllers,
    routes: [...fleetRoutes.values()]
      .map((r) => mergeRoutes(r.parts, r.service, r.route))
      .sort((a, b) => a.service.localeCompare(b.service) || a.route.localeCompare(b.route)),
    chat: fleetChat,
    chatSinceMs: chatSince,
    series: sumSeries ?? series([], now),
  };
}
