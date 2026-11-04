/**
 * A stand-in for one controller's game, for the demo (scripts/demo.ts): the admin service's GraphQL and
 * `/metrics` and the backend's `/admin/v1`, on one loopback port, answering as @dainty/fleet expects
 * (dainty-lass docs/admin-api.md). Its players, hunts and moderation live in memory, so a kick, ban or
 * access change in the dashboard shows on the next read. Signatures and the key are not checked: it
 * listens on 127.0.0.1 only and holds nothing real.
 */
import { ADMIN_OPS, type BackendAccount, type GamePlayer, type Hunt } from "@dainty/fleet";

export interface DemoGame {
  url: string;
  stop(): void;
}

const NAMES = [
  "Slayer_42",
  "Aetherheart",
  "Pangar_Pal",
  "Skraev_Bait",
  "IronMarrow",
  "Ramsgate_Rose",
  "Boreus_Breath",
  "Kharabak_Kid",
  "Quillshot",
  "Moonbound",
  "Riftstalker",
  "Ember_Wren",
  "Gnasher_Gnome",
  "Valomyr",
  "Torgadoro",
  "Nayzaga_Nik",
  "Shrike_Lane",
  "Frostfang",
  "Charrogg",
  "Stormclaw",
  "Drask_Dash",
  "Hellion_Hal",
  "Koshai_Kat",
  "Embermane",
  "Lumen_Lily",
  "Thrax_Tamer",
  "Agarus_Ace",
  "Malkarion",
  "Rezakiri",
  "Ostian_Owl",
  "Phaelanx",
  "Bertie_Bow",
  "Sable_Spear",
  "Willow_Wick",
  "Grim_Gale",
  "Orchid_Ox",
  "Pixel_Pike",
  "Tidal_Tess",
  "Hollow_Hux",
  "Copper_Cid",
];
const BEHEMOTHS = [
  "Shrike",
  "Skraev",
  "Kharabak",
  "Boreus",
  "Valomyr",
  "Hellion",
  "Charrogg",
  "Malkarion",
];
const MAPS = ["Frostbite Islands", "Thundering Wastes", "Brimstone", "Revelation Rock"];

const DAY = 86_400_000;

/** Controller `seed`'s i-th player, [userId, name]: the seed's chat and reports name the same people. */
export function demoPlayer(seed: number, i: number): [string, string] {
  const base = NAMES[((seed - 1) * 20 + i) % NAMES.length]!;
  return [`${seed}${String(1000 + i)}`, i < NAMES.length ? base : `${base}${i}`];
}
const iso = (ms: number) => new Date(ms).toISOString();

/** Deterministic per controller, so the same names come back after a restart. */
function rng(seed: number) {
  let s = seed;
  return () => {
    s = (s * 1103515245 + 12345) & 0x7fffffff;
    return s / 0x7fffffff;
  };
}

export function startDemoGame(opts: {
  port: number;
  name: string;
  mode: "PRIVATE" | "PUBLIC";
  /** Offset into NAMES, and the seed: each controller has its own players. */
  seed: number;
  players: number;
}): DemoGame {
  const random = rng(opts.seed + 7);
  const started = Date.now();
  const now = Date.now();
  const names = Array.from({ length: opts.players }, (_, i) => demoPlayer(opts.seed, i)[1]);
  const accounts: BackendAccount[] = names.map((name, i) => ({
    userId: demoPlayer(opts.seed, i)[0],
    displayName: name,
    email: `${name.toLowerCase()}@players.invalid`,
    approvalStatus: i % 13 === 5 ? "pending" : "approved",
    status: i % 17 === 3 ? "banned" : i % 19 === 7 ? "disabled" : "active",
    roles: i === 0 ? ["admin"] : i % 11 === 2 ? ["moderator"] : [],
    usernameSet: true,
    createdAt: iso(now - (60 - i) * DAY),
    lastLoginAt: iso(now - Math.floor(random() * 5 * DAY)),
  }));
  const players = new Map<string, GamePlayer>();
  for (const [i, a] of accounts.entries()) {
    const restricted =
      a.status === "banned"
        ? {
            kind: "BAN" as const,
            reason: "Cheating",
            since: iso(now - 4 * DAY),
            by: "Rook",
            until: null,
          }
        : a.status === "disabled"
          ? {
              kind: "DISABLED" as const,
              reason: null,
              since: iso(now - 9 * DAY),
              by: null,
              until: null,
            }
          : i % 23 === 9
            ? {
                kind: "TIMEOUT" as const,
                reason: "Spam",
                since: iso(now - 3_600_000),
                by: "Wren",
                until: iso(now + 2 * 3_600_000),
              }
            : null;
    players.set(a.displayName.toLowerCase(), {
      userId: a.userId,
      username: a.displayName,
      status: a.status === "banned" ? "BANNED" : a.status === "disabled" ? "DISABLED" : "ACTIVE",
      restriction: restricted,
      online: false,
      location: null,
      admin: a.roles.includes("admin"),
      createdAt: a.createdAt,
      lastLoginAt: a.lastLoginAt ?? null,
    });
  }
  const audit: {
    id: string;
    action: string;
    actorUserId?: string;
    targetUserId?: string;
    reason?: string;
    createdAt: string;
    oldState?: unknown;
    newState?: unknown;
  }[] = [];
  const hunts: Hunt[] = [];
  const maxHunts = 8;

  /** Who is on and where moves every few minutes, as a real server's does. */
  function shuffle() {
    const active = [...players.values()].filter((p) => p.status === "ACTIVE");
    const online = active.filter(() => random() < 0.45);
    for (const p of players.values()) {
      p.online = false;
      p.location = null;
    }
    hunts.length = 0;
    const huntCount = Math.min(maxHunts, Math.floor(online.length / 4));
    for (let h = 0; h < huntCount; h++) {
      const party = online.slice(h * 4, h * 4 + 4);
      const behemoth = BEHEMOTHS[Math.floor(random() * BEHEMOTHS.length)]!;
      const id = `hunt-${opts.seed}-${h}`;
      hunts.push({
        id,
        title: `${behemoth} (Heroic)`,
        map: MAPS[h % MAPS.length]!,
        behemoth,
        playerCount: party.length,
        maxPlayers: 4,
        players: party.map((p) => ({ userId: p.userId, username: p.username })),
        startedAt: iso(Date.now() - Math.floor(random() * 1_200_000)),
      });
      for (const p of party) {
        p.online = true;
        p.location = { kind: "HUNT", instanceId: id, title: `${behemoth} (Heroic)` };
      }
    }
    for (const p of online.slice(huntCount * 4)) {
      p.online = true;
      p.location =
        random() < 0.15
          ? { kind: "TRAINING", instanceId: "training-1", title: "Training grounds" }
          : { kind: "HUB", instanceId: "ramsgate-1", title: "Ramsgate" };
    }
  }
  shuffle();
  const shuffler = setInterval(shuffle, 180_000);

  // /metrics counters only grow, as a real exporter's do
  const counters = {
    requests: new Map<string, number>(),
    chat: { allowed: 0, flagged: 0, masked: 0, blocked: 0 },
  };
  const ROUTES = [
    ["gateway", "/api/v1/*", 40],
    ["backend", "/admin/v1/*", 6],
    ["backend", "/account/*", 14],
    ["content", "/runtime/*", 3],
    ["director", "/hunts/*", 9],
  ] as const;
  const BOUNDS = [5, 10, 25, 50, 100, 250, 500, 1000];

  function metrics(): string {
    const uptime = Math.floor((Date.now() - started) / 1000);
    const online = [...players.values()].filter((p) => p.online).length;
    const lines: string[] = [];
    const add = (name: string, labels: Record<string, string>, value: number) =>
      lines.push(
        `${name}{${Object.entries(labels)
          .map(([k, v]) => `${k}="${v}"`)
          .join(",")}} ${value}`,
      );
    for (const [component, phase, restarts] of [
      ["gateway", "ready", 0],
      ["backend", "ready", 0],
      ["director", "ready", 1],
      ["content", "ready", 0],
      ["chat", opts.seed === 3 ? "failed" : "ready", opts.seed === 3 ? 4 : 0],
    ] as const) {
      add("dainty_component_phase", { component, phase }, 1);
      add("dainty_component_restarts", { component }, restarts);
      add(
        "dainty_component_uptime_seconds",
        { component },
        phase === "failed" ? 0 : uptime + 3600 * opts.seed,
      );
    }
    const procs = [
      [4100 + opts.seed, "dainty-lass-server", "host", 6 + random() * 8, 420e6],
      [4200 + opts.seed, "mongod", "child", 2 + random() * 3, 310e6],
      ...hunts.map(
        (h, i) =>
          [
            5000 + i,
            `Dauntless-Win64-Shipping (${h.id})`,
            "child",
            18 + random() * 20,
            1.4e9,
          ] as const,
      ),
    ] as const;
    for (const [pid, name, role, cpu, memory] of procs) {
      add("dainty_process_cpu_percent", { pid: String(pid), name, role }, Number(cpu.toFixed(1)));
      add(
        "dainty_process_memory_bytes",
        { pid: String(pid), name, role },
        Math.round(memory * (0.9 + random() * 0.2)),
      );
      add("dainty_process_uptime_seconds", { pid: String(pid), name, role }, uptime);
    }
    add(
      "dainty_game_server_players",
      { instance: "ramsgate-1", kind: "hub" },
      online - hunts.reduce((n, h) => n + h.playerCount, 0),
    );
    add("dainty_game_server_max_players", { instance: "ramsgate-1", kind: "hub" }, 100);
    for (const h of hunts) {
      add("dainty_game_server_players", { instance: h.id, kind: "hunt" }, h.playerCount);
      add("dainty_game_server_max_players", { instance: h.id, kind: "hunt" }, h.maxPlayers);
    }
    for (const [service, route, rate] of ROUTES) {
      const key = `${service} ${route}`;
      const total = (counters.requests.get(key) ?? 0) + Math.round(rate * (0.5 + random()) * 15);
      counters.requests.set(key, total);
      const errors = Math.floor(total * (service === "director" ? 0.02 : 0.002));
      add("dainty_http_requests_total", { service, route, status: "2xx" }, total - errors);
      add("dainty_http_requests_total", { service, route, status: "5xx" }, errors);
      let cumulative = 0;
      for (const [i, le] of BOUNDS.entries()) {
        cumulative += Math.round(total * [0.2, 0.25, 0.25, 0.15, 0.08, 0.04, 0.02, 0.01][i]!);
        add(
          "dainty_http_request_duration_ms_bucket",
          { service, route, le: String(le) },
          Math.min(cumulative, total),
        );
      }
      add("dainty_http_request_duration_ms_bucket", { service, route, le: "+Inf" }, total);
      add(
        "dainty_http_request_duration_ms_sum",
        { service, route },
        total * (service === "content" ? 120 : 28),
      );
    }
    counters.chat.allowed += Math.round(30 * random());
    counters.chat.flagged += random() < 0.3 ? 1 : 0;
    counters.chat.masked += random() < 0.4 ? 1 : 0;
    counters.chat.blocked += random() < 0.15 ? 1 : 0;
    for (const [verdict, n] of Object.entries(counters.chat))
      add("dainty_chat_messages_total", { verdict }, n);
    add("dainty_chat_dashboard_posts_total", { result: "ok" }, counters.chat.allowed);
    add("dainty_players_online", {}, online);
    add("dainty_hunts_active", {}, hunts.length);
    add("dainty_hunts_max", {}, maxHunts);
    add("dainty_director_up", {}, 1);
    add("dainty_backend_metrics_up", {}, 1);
    return `${lines.join("\n")}\n`;
  }

  const OPS = new Map<string, string>(Object.entries(ADMIN_OPS).map(([op, query]) => [query, op]));
  const gqlError = (code: string, message: string) =>
    Response.json({ errors: [{ message, extensions: { code } }] });

  function admin(op: string, v: Record<string, unknown>): Response {
    const data = (value: unknown) => Response.json({ data: { [op]: value } });
    const online = [...players.values()].filter((p) => p.online).length;
    if (op === "serverStatus")
      return data({
        name: opts.name,
        mode: opts.mode,
        registration: opts.mode === "PUBLIC" ? "OPEN" : "INVITE",
        version: "2.0.0-beta",
        commit: "f891d92",
        uptimeSeconds: Math.floor((Date.now() - started) / 1000) + 86_400 * opts.seed,
        onlineCount: online,
        chat: opts.seed !== 3,
        gameData: "real",
      });
    if (op === "huntStatus")
      return data({
        maxHuntServers: maxHunts,
        running: hunts.length,
        free: maxHunts - hunts.length,
        hunts,
      });
    const p = players.get(String(v.u ?? "").toLowerCase());
    if (op === "player") return data(p ?? null);
    if (!p) return gqlError("NOT_FOUND", "no such player");
    if (p.admin && op !== "liftTimeout" && op !== "unban")
      return gqlError("PROTECTED_ACCOUNT", "an admin");
    const record = (action: string, reason?: string) =>
      audit.unshift({
        id: `a${audit.length + 1}`,
        action,
        targetUserId: p.userId,
        reason,
        createdAt: iso(Date.now()),
      });
    switch (op) {
      case "kick": {
        const closed = p.online ? 1 : 0;
        p.online = false;
        p.location = null;
        record("kick");
        return data({ userId: p.userId, username: p.username, connectionsClosed: closed });
      }
      case "timeout": {
        const until = typeof v.t === "string" ? v.t : iso(Date.now() + Number(v.m ?? 60) * 60_000);
        p.restriction = {
          kind: "TIMEOUT",
          reason: String(v.r ?? ""),
          since: iso(Date.now()),
          by: "dashboard",
          until,
        };
        p.online = false;
        p.location = null;
        record("timeout", String(v.r ?? ""));
        return data(p);
      }
      case "liftTimeout":
        if (p.restriction?.kind !== "TIMEOUT") return gqlError("NOT_TIMED_OUT", "not timed out");
        p.restriction = null;
        record("lift_timeout");
        return data(p);
      case "ban":
        if (p.status === "BANNED") return gqlError("ALREADY_BANNED", "already banned");
        p.status = "BANNED";
        p.restriction = {
          kind: "BAN",
          reason: String(v.r ?? ""),
          since: iso(Date.now()),
          by: "dashboard",
          until: null,
        };
        p.online = false;
        p.location = null;
        setAccount(p.userId, { status: "banned" });
        record("ban", String(v.r ?? ""));
        return data(p);
      case "unban":
        if (p.status !== "BANNED") return gqlError("NOT_BANNED", "not banned");
        p.status = "ACTIVE";
        p.restriction = null;
        setAccount(p.userId, { status: "active" });
        record("unban", typeof v.r === "string" ? v.r : undefined);
        return data(p);
    }
    return gqlError("BAD_INPUT", `unknown operation ${op}`);
  }

  function setAccount(userId: string, change: Partial<BackendAccount>) {
    const a = accounts.find((x) => x.userId === userId);
    if (a) Object.assign(a, change);
    return a;
  }

  async function backend(request: Request, url: URL): Promise<Response> {
    const path = url.pathname;
    if (request.method === "GET" && path === "/admin/v1/online-players") {
      const on = [...players.values()].filter((p) => p.online);
      return Response.json({
        count: on.length,
        players: on.map((p) => ({ userId: p.userId, displayName: p.username })),
        observedAt: iso(Date.now()),
      });
    }
    if (request.method === "GET" && path === "/admin/v1/players") {
      const ids = url.searchParams.get("ids");
      if (ids) {
        const wanted = new Set(ids.split(","));
        return Response.json({ accounts: accounts.filter((a) => wanted.has(a.userId)) });
      }
      const search = url.searchParams.get("search")?.toLowerCase() ?? "";
      const status = url.searchParams.get("status");
      const page = Math.max(1, Number(url.searchParams.get("page") ?? 1));
      const limit = Math.min(100, Math.max(1, Number(url.searchParams.get("limit") ?? 25)));
      const found = accounts.filter(
        (a) =>
          (!search || a.displayName.toLowerCase().includes(search) || a.userId.includes(search)) &&
          (!status || a.status === status || a.approvalStatus === status),
      );
      return Response.json({
        accounts: found.slice((page - 1) * limit, page * limit),
        total: found.length,
        page,
        limit,
      });
    }
    if (request.method === "GET" && path === "/admin/v1/audit") {
      const target = url.searchParams.get("targetUserId");
      const records = audit.filter((r) => !target || r.targetUserId === target);
      return Response.json({ records: records.slice(0, 50), total: records.length });
    }
    const access = path.match(/^\/admin\/v1\/players\/([^/]+)\/(access|roles)$/);
    if (request.method === "PATCH" && access) {
      const userId = decodeURIComponent(access[1]!);
      const body = (await request.json().catch(() => ({}))) as Record<string, unknown>;
      const before = accounts.find((a) => a.userId === userId);
      if (!before) return Response.json({ error: { code: "NOT_FOUND" } }, { status: 404 });
      const old = { ...before };
      const change: Partial<BackendAccount> =
        access[2] === "roles"
          ? {
              roles: Array.isArray(body.roles) ? body.roles.map(String) : before.roles,
              rolesUpdatedAt: iso(Date.now()),
            }
          : {
              ...(typeof body.status === "string"
                ? { status: body.status as BackendAccount["status"] }
                : {}),
              ...(typeof body.approvalStatus === "string"
                ? {
                    approvalStatus: body.approvalStatus as BackendAccount["approvalStatus"],
                    approvalUpdatedAt: iso(Date.now()),
                  }
                : {}),
            };
      const account = setAccount(userId, change)!;
      const p = [...players.values()].find((x) => x.userId === userId);
      if (p && change.status)
        p.status =
          change.status === "banned"
            ? "BANNED"
            : change.status === "disabled"
              ? "DISABLED"
              : "ACTIVE";
      audit.unshift({
        id: `a${audit.length + 1}`,
        action: access[2] === "roles" ? "set_roles" : "set_access",
        targetUserId: userId,
        reason: typeof body.reason === "string" ? body.reason : undefined,
        createdAt: iso(Date.now()),
        oldState: old,
        newState: account,
      });
      return Response.json({ account });
    }
    return Response.json({ error: { code: "NOT_FOUND" } }, { status: 404 });
  }

  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: opts.port,
    async fetch(request) {
      const url = new URL(request.url);
      if (url.pathname === "/metrics")
        return new Response(metrics(), { headers: { "Content-Type": "text/plain" } });
      if (url.pathname === "/graphql" && request.method === "POST") {
        const body = (await request.json().catch(() => null)) as {
          query?: string;
          variables?: Record<string, unknown>;
        } | null;
        const op = OPS.get(body?.query ?? "");
        if (!op) return gqlError("BAD_INPUT", "not one of the dashboard's operations");
        return admin(op, body?.variables ?? {});
      }
      if (url.pathname.startsWith("/admin/v1/")) return backend(request, url);
      return new Response("not found", { status: 404 });
    },
  });

  return {
    url: `http://127.0.0.1:${server.port}`,
    stop() {
      clearInterval(shuffler);
      server.stop(true);
    },
  };
}
