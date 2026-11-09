import { afterAll, beforeEach, describe, expect, test } from "bun:test";
import { GET } from "@/app/api/public/signup/route.dashboard";
import type { Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { gamesReport, KEPT_EVENTS, refreshGames, resetGames } from "@/lib/fleet/games";
import { updateSettings } from "@/lib/settings-change";
import { identity, signedIn } from "./helpers";

describe("GET /api/public/signup", () => {
  let owner: Staff;
  const read = async () => (await GET()).json();

  test("off by default, with no link", async () => {
    expect(await read()).toEqual({ enabled: false, url: null });
  });

  test("answers the link only while account creation is on", async () => {
    owner = signedIn(identity({ name: "owner", discordId: "100000000000000001" }));
    updateSettings(owner, { signupUrl: "https://dash.example/signup" });
    expect(await read()).toEqual({ enabled: false, url: null });
    updateSettings(owner, { signupEnabled: true });
    expect(await read()).toEqual({ enabled: true, url: "https://dash.example/signup" });
    expect(Object.keys(await read())).toEqual(["enabled", "url"]);
  });

  test("on without a link reads as off", async () => {
    updateSettings(owner, { signupUrl: "" });
    expect(await read()).toEqual({ enabled: false, url: null });
  });

  test("a link the bot would drop is refused", () => {
    for (const url of [
      "javascript:alert(1)",
      "https://u:p@dash.example/",
      "not a url",
      `https://x.example/${"a".repeat(600)}`,
    ])
      expect(() => updateSettings(owner, { signupUrl: url })).toThrow(DomainError);
  });
});

describe("the game report", () => {
  const SERVER = {
    game: "1.14.7",
    id: "hub-1",
    kind: "HUB",
    title: "Ramsgate",
    map: "Ramsgate",
    host: "127.0.0.1",
    port: 7777,
    players: 3,
    maxPlayers: 100,
    ready: true,
    startedAt: null,
  };
  const events = Array.from({ length: 60 }, (_, i) => ({
    seq: i + 1,
    atMs: 1_000 + i,
    name: "game.server.up",
    source: "rustic-isles",
  }));
  let fleet = true;
  const seen: { query: string; variables: Record<string, unknown> }[] = [];
  const admin = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(req) {
      const body = (await req.json()) as { query: string; variables: Record<string, unknown> };
      seen.push(body);
      if (body.query.includes("installs"))
        return fleet
          ? Response.json({ data: { installs: [] } })
          : Response.json({
              errors: [{ message: "not a fleet", extensions: { code: "NO_FLEET" } }],
            });
      if (body.query.includes("events")) {
        const after = Number(body.variables.a ?? 0);
        return Response.json({
          data: { events: events.filter((e) => e.seq > after).slice(0, 1000) },
        });
      }
      if (body.query.includes("availability"))
        return Response.json({
          data: {
            availability: { game: "1.14.7", up: true, reason: null, hub: SERVER, freeHunts: 4 },
          },
        });
      return Response.json({ data: { gameServers: [SERVER] } });
    },
  });
  const cfg = { adminApiUrl: `http://127.0.0.1:${admin.port}/`, adminApiKey: "k", backendUrl: "" };

  beforeEach(() => resetGames());
  afterAll(() => admin.stop(true));

  test("reads servers, availability, installs and the newest events", async () => {
    expect(gamesReport()).toBeNull();
    const r = await refreshGames(cfg);
    expect(r.gameServers?.[0]?.title).toBe("Ramsgate");
    expect(r.availability?.up).toBe(true);
    expect(r.installs).toEqual([]);
    expect(r.events).toHaveLength(KEPT_EVENTS);
    expect(r.events[0]?.seq).toBe(60);
    expect(gamesReport()).toBe(r);
  });

  test("a later read asks only for events after the last one", async () => {
    await refreshGames(cfg);
    seen.length = 0;
    await refreshGames(cfg);
    expect(seen.find((s) => s.query.includes("events"))?.variables).toEqual({ a: 60 });
  });

  test("NO_FLEET leaves installs empty, and a dead admin API leaves servers unknown", async () => {
    fleet = false;
    expect((await refreshGames(cfg)).installs).toBeNull();
    fleet = true;
    const dead = await refreshGames({ ...cfg, adminApiUrl: "http://127.0.0.1:1/" });
    expect(dead.gameServers).toBeNull();
    expect(dead.availability).toBeNull();
  });
});
