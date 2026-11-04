import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  histogramQuantile,
  metricsUrl,
  parsePrometheus,
  parseSampleLine,
  sanitizeSummary,
  summarize,
} from "../src";

// A scrape shaped as the admin service renders it (service_kit::metrics::render), with the backend's
// own families appended.
const fixture = readFileSync(join(import.meta.dir, "fixtures", "metrics.txt"), "utf8");

describe("the text format", () => {
  test("every sample line is read, comments are not", () => {
    const samples = parsePrometheus(fixture);
    const lines = fixture.split("\n").filter((l) => l.trim() && !l.startsWith("#"));
    expect(samples.length).toBe(lines.length);
  });

  test("label values unescape, and a timestamp is ignored", () => {
    expect(
      parseSampleLine(
        String.raw`rustic_isles_http_requests_total{route="/launcher/v1/me",note="say \"hi\"\\ok\nbye"} 42 1793500000000`,
      ),
    ).toEqual({
      name: "rustic_isles_http_requests_total",
      labels: { route: "/launcher/v1/me", note: 'say "hi"\\ok\nbye' },
      value: 42,
    });
  });

  test("special values, exponents and spacing", () => {
    expect(parseSampleLine("a 1.5e+03")?.value).toBe(1500);
    expect(parseSampleLine("a +Inf")?.value).toBe(Number.POSITIVE_INFINITY);
    expect(parseSampleLine("a NaN")?.value).toBeNaN();
    expect(parseSampleLine('a{ x = "1" , y="2", } -3')).toEqual({
      name: "a",
      labels: { x: "1", y: "2" },
      value: -3,
    });
    expect(parseSampleLine("a{} 7")).toEqual({ name: "a", labels: {}, value: 7 });
  });

  test("a malformed line is dropped, not fatal", () => {
    for (const line of ['a{x="1} 2', "a{x=1} 2", "a one", "1abc 2", "a 1 2 3", 'a{x="1" 2'])
      expect(parseSampleLine(line)).toBeNull();
    expect(parsePrometheus("good 1\nbad{ 2\nalso_good 3\n").map((s) => s.name)).toEqual([
      "good",
      "also_good",
    ]);
  });
});

describe("the summary", () => {
  const s = summarize(parsePrometheus(fixture), 1_000);

  test("server counts and the upstreams' health", () => {
    expect(s).toMatchObject({
      at: 1_000,
      players: 15,
      huntsActive: 1,
      huntsMax: 8,
      directorUp: true,
      backendMetricsUp: true,
    });
  });

  test("each component's phase, restarts and uptime", () => {
    expect(s.components).toEqual([
      { name: "backend", phase: "ready", restarts: 0, uptimeSeconds: 86_400 },
      { name: "director", phase: "ready", restarts: 1, uptimeSeconds: 3_600 },
      { name: "gateway", phase: "ready", restarts: 0, uptimeSeconds: 86_410 },
      { name: "mongod", phase: "failed", restarts: 4, uptimeSeconds: 0 },
    ]);
  });

  test("processes, the host first", () => {
    expect(s.processes.map((p) => [p.role, p.name, p.pid])).toEqual([
      ["host", "controller", 4120],
      ["child", "DauntlessServer-Win64-Shipping.exe", 5532],
      ["child", "mongod", 4200],
    ]);
    expect(s.processes[1]).toMatchObject({
      cpuPercent: 87.25,
      memoryBytes: 2_147_483_648,
      uptimeSeconds: 600,
    });
  });

  test("game servers with their caps", () => {
    expect(s.gameServers).toEqual([
      { instance: "hub-1", kind: "city", players: 12, maxPlayers: 50 },
      { instance: "hunt-7f3a", kind: "hunt", players: 3, maxPlayers: 4 },
    ]);
  });

  test("requests summed over methods and status classes, with the histogram", () => {
    const gw = s.http.find((r) => r.service === "gateway" && r.route === "backend")!;
    expect(gw).toMatchObject({ requests: 1000, errors: 20, count: 1000, sumMs: 21_234.75 });
    expect(gw.bounds).toEqual([5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000]);
    expect(gw.buckets[0]).toBe(100);
    expect(s.http.map((r) => `${r.service}/${r.route}`)).toEqual([
      "admin/all",
      "gateway/backend",
      "gateway/refused",
    ]);
  });

  test("chat by verdict across channels, and the dashboard posts by result", () => {
    expect(s.chat).toEqual({ allowed: 1500, flagged: 7, masked: 4, blocked: 2 });
    expect(s.dashboardPosts).toEqual({ ok: 412, failed: 3, refused: 1 });
  });

  test("an empty scrape is an empty summary", () => {
    expect(summarize([], 5)).toMatchObject({
      players: null,
      directorUp: null,
      components: [],
      http: [],
      chat: {},
    });
  });

  test("survives JSON and the dashboard's checks unchanged", () => {
    expect(sanitizeSummary(JSON.parse(JSON.stringify(s)))).toEqual(s);
    expect(sanitizeSummary("nope")).toBeNull();
    const odd = sanitizeSummary({
      components: [{ name: "x".repeat(500), phase: "exploded", restarts: "9" }],
      http: [{ service: "a", bounds: [1, 2], buckets: [1] }],
      chat: { allowed: Number.NaN },
    })!;
    expect(odd.components[0]).toEqual({
      name: "x".repeat(128),
      phase: "unknown",
      restarts: 0,
      uptimeSeconds: 0,
    });
    expect(odd.http[0]).toMatchObject({ bounds: [], buckets: [] });
    expect(odd.chat).toEqual({ allowed: 0 });
  });
});

describe("quantiles from buckets", () => {
  const bounds = [5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000];
  const buckets = [100, 400, 800, 900, 950, 990, 1000, 1000, 1000, 1000];

  test("interpolated inside the bucket, as histogram_quantile does", () => {
    // The 500th of 1000 is the 100th of 400 in (10, 25]: 10 + 15 * 100 / 400.
    expect(histogramQuantile(0.5, bounds, buckets, 1000)).toBeCloseTo(13.75);
    // The 950th is the last in (50, 100].
    expect(histogramQuantile(0.95, bounds, buckets, 1000)).toBeCloseTo(100);
    expect(histogramQuantile(0.05, bounds, buckets, 1000)).toBeCloseTo(2.5);
  });

  test("past the last bound is that bound; nothing observed is null", () => {
    expect(histogramQuantile(0.99, [5, 10], [0, 0], 10)).toBe(10);
    expect(
      histogramQuantile(
        0.5,
        bounds,
        bounds.map(() => 0),
        0,
      ),
    ).toBeNull();
  });
});

test("/metrics sits beside the admin GraphQL", () => {
  expect(metricsUrl({ adminApiUrl: "http://127.0.0.1:61003/graphql", backendUrl: "" })).toBe(
    "http://127.0.0.1:61003/metrics",
  );
});
