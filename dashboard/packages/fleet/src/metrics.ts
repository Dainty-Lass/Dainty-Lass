import type { GameConfig } from "./game";
import { type PromSample, parsePrometheus } from "./prometheus";

/**
 * What the dashboard keeps of a controller's `/metrics` (dainty-lass docs/admin-api.md, "Metrics"): a
 * few kilobytes, made the same way by the dashboard for its own host and by an agent for its own, and
 * sent with the status. Counters stay cumulative here; the dashboard turns them into rates from its
 * ring of samples, so a missed heartbeat costs resolution, not correctness.
 */

export const COMPONENT_PHASES = [
  "stopped",
  "starting",
  "ready",
  "stopping",
  "failed",
  "disabled",
] as const;
export type ComponentPhase = (typeof COMPONENT_PHASES)[number] | "unknown";

export interface ComponentMetrics {
  name: string;
  phase: ComponentPhase;
  restarts: number;
  uptimeSeconds: number;
}

export interface ProcessMetrics {
  pid: number;
  name: string;
  /** `host` is the controller itself; `child` is everything it started: game servers, mongod, turn-rs. */
  role: "host" | "child";
  cpuPercent: number;
  memoryBytes: number;
  uptimeSeconds: number;
}

export interface GameServerMetrics {
  instance: string;
  kind: string;
  players: number;
  maxPlayers: number;
}

/** One service and route class, every method and status class summed. */
export interface RouteMetrics {
  service: string;
  route: string;
  requests: number;
  /** Answers in the 5xx class. */
  errors: number;
  /** Finite upper bounds of the latency histogram in ms, ascending (JSON has no +Inf). */
  bounds: number[];
  /** Cumulative counts per bound. */
  buckets: number[];
  /** Every timed request: the +Inf bucket. */
  count: number;
  sumMs: number;
}

export interface MetricsSummary {
  at: number;
  players: number | null;
  huntsActive: number | null;
  huntsMax: number | null;
  directorUp: boolean | null;
  backendMetricsUp: boolean | null;
  components: ComponentMetrics[];
  processes: ProcessMetrics[];
  gameServers: GameServerMetrics[];
  http: RouteMetrics[];
  /** Chat lines by the filter's verdict, every channel summed; cumulative. */
  chat: Record<string, number>;
  /** Posts from the game to this dashboard's ingest, by result; cumulative. */
  dashboardPosts: Record<string, number>;
}

const finite = (n: number) => (Number.isFinite(n) ? n : 0);

function gauge(samples: PromSample[], name: string): number | null {
  const s = samples.find((x) => x.name === name);
  return s ? finite(s.value) : null;
}

function sumBy(samples: PromSample[], name: string, label: string): Record<string, number> {
  const out: Record<string, number> = {};
  for (const s of samples) {
    if (s.name !== name) continue;
    const key = s.labels[label] ?? "";
    out[key] = (out[key] ?? 0) + finite(s.value);
  }
  return out;
}

/** The parsed scrape, reduced to what the dashboard shows. */
export function summarize(samples: PromSample[], at = Date.now()): MetricsSummary {
  const components = new Map<string, ComponentMetrics>();
  const component = (name: string) => {
    let c = components.get(name);
    if (!c) {
      c = { name, phase: "unknown", restarts: 0, uptimeSeconds: 0 };
      components.set(name, c);
    }
    return c;
  };
  const processes = new Map<number, ProcessMetrics>();
  const servers = new Map<string, GameServerMetrics>();
  const routes = new Map<string, RouteMetrics & { le: Map<number, number> }>();
  const route = (labels: Record<string, string>) => {
    const service = labels.service ?? "";
    const name = labels.route ?? "";
    const key = `${service}\u0000${name}`;
    let r = routes.get(key);
    if (!r) {
      r = {
        service,
        route: name,
        requests: 0,
        errors: 0,
        bounds: [],
        buckets: [],
        count: 0,
        sumMs: 0,
        le: new Map(),
      };
      routes.set(key, r);
    }
    return r;
  };

  for (const s of samples) {
    const l = s.labels;
    switch (s.name) {
      case "dainty_component_phase":
        if (
          l.component &&
          s.value === 1 &&
          (COMPONENT_PHASES as readonly string[]).includes(l.phase ?? "")
        )
          component(l.component).phase = l.phase as ComponentPhase;
        else if (l.component) component(l.component);
        break;
      case "dainty_component_restarts":
        if (l.component) component(l.component).restarts = finite(s.value);
        break;
      case "dainty_component_uptime_seconds":
        if (l.component) component(l.component).uptimeSeconds = finite(s.value);
        break;
      case "dainty_process_cpu_percent":
      case "dainty_process_memory_bytes":
      case "dainty_process_uptime_seconds": {
        const pid = Number(l.pid);
        if (!Number.isInteger(pid)) break;
        let p = processes.get(pid);
        if (!p) {
          p = {
            pid,
            name: l.name ?? "",
            role: l.role === "host" ? "host" : "child",
            cpuPercent: 0,
            memoryBytes: 0,
            uptimeSeconds: 0,
          };
          processes.set(pid, p);
        }
        if (s.name === "dainty_process_cpu_percent") p.cpuPercent = finite(s.value);
        else if (s.name === "dainty_process_memory_bytes") p.memoryBytes = finite(s.value);
        else p.uptimeSeconds = finite(s.value);
        break;
      }
      case "dainty_game_server_players":
      case "dainty_game_server_max_players": {
        if (!l.instance) break;
        let g = servers.get(l.instance);
        if (!g) {
          g = { instance: l.instance, kind: l.kind ?? "", players: 0, maxPlayers: 0 };
          servers.set(l.instance, g);
        }
        if (s.name === "dainty_game_server_players") g.players = finite(s.value);
        else g.maxPlayers = finite(s.value);
        break;
      }
      case "dainty_http_requests_total": {
        const r = route(l);
        r.requests += finite(s.value);
        if (l.status === "5xx") r.errors += finite(s.value);
        break;
      }
      case "dainty_http_request_duration_ms_bucket": {
        const le = l.le === "+Inf" ? Number.POSITIVE_INFINITY : Number(l.le);
        if (Number.isNaN(le)) break;
        const r = route(l);
        if (le === Number.POSITIVE_INFINITY) r.count += finite(s.value);
        else r.le.set(le, (r.le.get(le) ?? 0) + finite(s.value));
        break;
      }
      case "dainty_http_request_duration_ms_sum":
        route(l).sumMs += finite(s.value);
        break;
    }
  }

  const http = [...routes.values()].map(({ le, ...r }) => {
    const bounds = [...le.keys()].sort((a, b) => a - b);
    return { ...r, bounds, buckets: bounds.map((b) => le.get(b) ?? 0) };
  });
  const up = (name: string) => {
    const v = gauge(samples, name);
    return v === null ? null : v >= 1;
  };
  return {
    at,
    players: gauge(samples, "dainty_players_online"),
    huntsActive: gauge(samples, "dainty_hunts_active"),
    huntsMax: gauge(samples, "dainty_hunts_max"),
    directorUp: up("dainty_director_up"),
    backendMetricsUp: up("dainty_backend_metrics_up"),
    components: [...components.values()].sort((a, b) => a.name.localeCompare(b.name)),
    processes: [...processes.values()].sort((a, b) =>
      a.role === b.role
        ? a.name.localeCompare(b.name) || a.pid - b.pid
        : a.role === "host"
          ? -1
          : 1,
    ),
    gameServers: [...servers.values()].sort(
      (a, b) => a.kind.localeCompare(b.kind) || a.instance.localeCompare(b.instance),
    ),
    http: http.sort((a, b) => a.service.localeCompare(b.service) || a.route.localeCompare(b.route)),
    chat: sumBy(samples, "dainty_chat_messages_total", "verdict"),
    dashboardPosts: sumBy(samples, "dainty_chat_dashboard_posts_total", "result"),
  };
}

/**
 * Prometheus' `histogram_quantile`: the bucket holding the q-th observation, interpolated linearly
 * inside it. An answer past the last finite bound is that bound. Null with nothing observed.
 */
export function histogramQuantile(
  q: number,
  bounds: number[],
  buckets: number[],
  count: number,
): number | null {
  if (count <= 0 || bounds.length !== buckets.length) return null;
  const rank = q * count;
  for (let i = 0; i < bounds.length; i++) {
    const cumulative = buckets[i]!;
    if (cumulative < rank) continue;
    const upper = bounds[i]!;
    const lower = i === 0 ? 0 : bounds[i - 1]!;
    const below = i === 0 ? 0 : buckets[i - 1]!;
    const inBucket = cumulative - below;
    if (inBucket <= 0) return upper;
    return lower + (upper - lower) * ((rank - below) / inBucket);
  }
  return bounds.at(-1) ?? null;
}

/** The admin service serves `/metrics` beside its GraphQL, on the same key. */
export function metricsUrl(cfg: GameConfig): string {
  return new URL("/metrics", cfg.adminApiUrl).toString();
}

const MAX_BYTES = 4 * 1024 * 1024;

/** This host's `/metrics`, summarised; null when it is not configured or not answering. */
export async function scrapeMetrics(cfg: GameConfig): Promise<MetricsSummary | null> {
  if (!cfg.adminApiKey) return null;
  try {
    const response = await fetch(metricsUrl(cfg), {
      headers: { Authorization: `Bearer ${cfg.adminApiKey}` },
      signal: AbortSignal.timeout(10_000),
    });
    if (!response.ok) return null;
    const text = await response.text();
    if (text.length > MAX_BYTES) return null;
    return summarize(parsePrometheus(text));
  } catch {
    return null;
  }
}

// What an agent sends is checked field by field, and capped, before the dashboard keeps it.

const str = (v: unknown, max = 128) => (typeof v === "string" ? v.slice(0, max) : "");
const num = (v: unknown) => (typeof v === "number" && Number.isFinite(v) ? v : 0);
const nullableNum = (v: unknown) => (typeof v === "number" && Number.isFinite(v) ? v : null);
const nullableBool = (v: unknown) => (typeof v === "boolean" ? v : null);
const list = (v: unknown, max: number): Record<string, unknown>[] =>
  Array.isArray(v)
    ? v
        .slice(0, max)
        .filter((x): x is Record<string, unknown> => typeof x === "object" && x !== null)
    : [];

function counts(v: unknown): Record<string, number> {
  const out: Record<string, number> = {};
  if (typeof v !== "object" || v === null) return out;
  for (const [k, n] of Object.entries(v).slice(0, 32)) out[k.slice(0, 32)] = num(n);
  return out;
}

export function sanitizeSummary(value: unknown): MetricsSummary | null {
  if (typeof value !== "object" || value === null) return null;
  const v = value as Record<string, unknown>;
  return {
    at: num(v.at),
    players: nullableNum(v.players),
    huntsActive: nullableNum(v.huntsActive),
    huntsMax: nullableNum(v.huntsMax),
    directorUp: nullableBool(v.directorUp),
    backendMetricsUp: nullableBool(v.backendMetricsUp),
    components: list(v.components, 100).map((c) => ({
      name: str(c.name),
      phase: (COMPONENT_PHASES as readonly string[]).includes(c.phase as string)
        ? (c.phase as ComponentPhase)
        : "unknown",
      restarts: num(c.restarts),
      uptimeSeconds: num(c.uptimeSeconds),
    })),
    processes: list(v.processes, 200).map((p) => ({
      pid: num(p.pid),
      name: str(p.name),
      role: p.role === "host" ? "host" : "child",
      cpuPercent: num(p.cpuPercent),
      memoryBytes: num(p.memoryBytes),
      uptimeSeconds: num(p.uptimeSeconds),
    })),
    gameServers: list(v.gameServers, 200).map((g) => ({
      instance: str(g.instance, 64),
      kind: str(g.kind, 16),
      players: num(g.players),
      maxPlayers: num(g.maxPlayers),
    })),
    http: list(v.http, 64).map((r) => {
      const bounds = (Array.isArray(r.bounds) ? r.bounds.slice(0, 32) : []).map(num);
      const buckets = (Array.isArray(r.buckets) ? r.buckets.slice(0, 32) : []).map(num);
      const ok = bounds.length === buckets.length;
      return {
        service: str(r.service, 64),
        route: str(r.route, 64),
        requests: num(r.requests),
        errors: num(r.errors),
        bounds: ok ? bounds : [],
        buckets: ok ? buckets : [],
        count: num(r.count),
        sumMs: num(r.sumMs),
      };
    }),
    chat: counts(v.chat),
    dashboardPosts: counts(v.dashboardPosts),
  };
}
