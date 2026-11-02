import { beforeAll, beforeEach, describe, expect, test } from "bun:test";
import { type AgentStatus, type MetricsSummary, parsePrometheus, summarize } from "@dainty/fleet";
import type { Staff } from "@/lib/db/schema";
import { createController, getController, pairAgent } from "@/lib/fleet/controllers";
import { forgetMetrics, performance, recordMetrics } from "@/lib/fleet/metrics";
import { attach } from "@/lib/fleet/registry";
import { fleetPerformance, recordStatus, viewOf } from "@/lib/fleet/status";
import { identity, signedIn } from "./helpers";

let owner: Staff;
beforeAll(() => {
  owner = signedIn(identity({ name: "owner", discordId: "100000000000000001" }));
});
beforeEach(() => forgetMetrics());

const BOUNDS = [5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000];

/** A scrape where `requests` requests took 20 ms each (bucket le=25), `errors` of them 5xx. */
function scrape(requests: number, errors: number, chat: Record<string, number>): MetricsSummary {
  const lines = [
    `dainty_http_requests_total{method="GET",route="backend",service="gateway",status="2xx"} ${requests - errors}`,
    `dainty_http_requests_total{method="GET",route="backend",service="gateway",status="5xx"} ${errors}`,
    ...BOUNDS.map(
      (le) =>
        `dainty_http_request_duration_ms_bucket{le="${le}",route="backend",service="gateway"} ${le >= 25 ? requests : 0}`,
    ),
    `dainty_http_request_duration_ms_bucket{le="+Inf",route="backend",service="gateway"} ${requests}`,
    `dainty_http_request_duration_ms_sum{route="backend",service="gateway"} ${requests * 20}`,
    `dainty_component_phase{component="gateway",phase="ready"} 1`,
    `dainty_process_cpu_percent{name="controller",pid="1",role="host"} 10`,
    `dainty_process_memory_bytes{name="controller",pid="1",role="host"} 1000`,
    ...Object.entries(chat).map(
      ([verdict, n]) => `dainty_chat_messages_total{channel="city",verdict="${verdict}"} ${n}`,
    ),
  ];
  return summarize(parsePrometheus(lines.join("\n")));
}

const input = (id: string, summary: MetricsSummary) => ({
  id,
  name: id,
  state: "online" as const,
  controller: { status: { metrics: summary } as unknown as AgentStatus },
});

describe("rates from the ring", () => {
  test("requests a second, the 5xx share and latency over the window", () => {
    const now = 10 * 60_000;
    recordMetrics("a", scrape(100, 0, {}), now - 120_000);
    recordMetrics("a", scrape(160, 3, {}), now - 60_000);
    const last = scrape(220, 6, {});
    recordMetrics("a", last, now);
    const perf = performance([input("a", last)], { chat: false, now });
    const [route] = perf.controllers[0]!.routes;
    expect(route).toMatchObject({ service: "gateway", route: "backend" });
    expect(route!.perSecond).toBeCloseTo(1); // 120 requests over 120 s
    expect(route!.errorShare).toBeCloseTo(6 / 120);
    // Every request is in (10, 25]: the median interpolates to its middle.
    expect(route!.p50).toBeCloseTo(17.5);
    expect(perf.controllers[0]!.overall!.perSecond).toBeCloseTo(1);
    expect(perf.routes[0]!.perSecond).toBeCloseTo(1);
  });

  test("a restart's drop in the counters counts as new requests, not negative ones", () => {
    const now = 10 * 60_000;
    recordMetrics("a", scrape(500, 0, {}), now - 60_000);
    const after = scrape(30, 0, {});
    recordMetrics("a", after, now);
    const [route] = performance([input("a", after)], { chat: false, now }).controllers[0]!.routes;
    expect(route!.perSecond).toBeCloseTo(0.5);
  });

  test("one sample is no rate yet", () => {
    const s = scrape(10, 0, {});
    recordMetrics("a", s, 1_000);
    const perf = performance([input("a", s)], { chat: false, now: 1_000 });
    expect(perf.controllers[0]!.routes[0]!.perSecond).toBeNull();
    expect(perf.controllers[0]!.overall).toBeNull();
  });

  test("samples older than an hour fall out", () => {
    const now = 3 * 3_600_000;
    recordMetrics("a", scrape(0, 0, {}), now - 2 * 3_600_000);
    recordMetrics("a", scrape(10, 0, {}), now - 30 * 60_000);
    recordMetrics("a", scrape(20, 0, {}), now);
    const perf = performance([input("a", scrape(20, 0, {}))], { chat: true, now });
    expect(perf.chatSinceMs).toBe(now - 30 * 60_000);
  });

  test("the hour's series has a point a minute, summed over controllers", () => {
    const now = 60 * 60_000;
    for (const id of ["a", "b"]) {
      recordMetrics(id, scrape(0, 0, {}), now - 60_000);
      recordMetrics(id, scrape(30, 0, {}), now - 30_000);
    }
    const perf = performance([input("a", scrape(30, 0, {})), input("b", scrape(30, 0, {}))], {
      chat: false,
      now,
    });
    expect(perf.series).toHaveLength(60);
    expect(perf.series.reduce((n, p) => n + p.requests, 0)).toBe(60);
    expect(perf.series.at(-2)!.cpu).toBe(20);
  });
});

describe("chat counts", () => {
  test("by verdict over the ring, only for readers who may see chat", () => {
    const now = 10 * 60_000;
    recordMetrics("a", scrape(0, 0, { allowed: 10, flagged: 1 }), now - 60_000);
    const last = scrape(0, 0, { allowed: 25, flagged: 3, blocked: 1 });
    recordMetrics("a", last, now);
    expect(performance([input("a", last)], { chat: true, now }).chat).toEqual({
      allowed: 15,
      flagged: 2,
      blocked: 1,
    });
    const hidden = performance([input("a", last)], { chat: false, now });
    expect(hidden.chat).toBeNull();
    expect(hidden.controllers[0]!.chat).toBeNull();
  });
});

describe("through the status", () => {
  test("a controller's report feeds the ring; lists of controllers do not carry the summary", () => {
    const { controller, code } = createController(owner, { name: "Metrics" });
    pairAgent(code, "0.1.0");
    const detach = attach(controller.id, { send: () => {}, close: () => {} });
    const status = (metrics: MetricsSummary): AgentStatus => ({
      protocol: 1,
      version: "0.1.0",
      capabilities: ["admin", "backend", "metrics"],
      at: Date.now(),
      adminApi: "ok",
      backend: "ok",
      server: null,
      hunts: null,
      online: 0,
      metrics,
    });
    recordStatus(controller.id, status(scrape(10, 0, { allowed: 1 })));
    recordStatus(controller.id, status(scrape(20, 0, { allowed: 4 })));
    expect(viewOf(getController(controller.id)!).status?.metrics).toBeUndefined();
    const perf = fleetPerformance([controller.id], true);
    expect(perf.controllers[0]!.summary?.components[0]).toMatchObject({
      name: "gateway",
      phase: "ready",
    });
    expect(perf.chat).toEqual({ allowed: 3 });
    detach();
  });

  test("an agent without metrics shows as not reporting", () => {
    const { controller, code } = createController(owner, { name: "Old agent" });
    pairAgent(code, "0.1.0");
    recordStatus(controller.id, {
      protocol: 1,
      version: "0.0.9",
      capabilities: ["admin", "backend"],
      at: Date.now(),
      adminApi: "ok",
      backend: "ok",
      server: null,
      hunts: null,
      online: 0,
    });
    expect(fleetPerformance([controller.id], false).controllers[0]!.summary).toBeNull();
  });
});
