/**
 * The whole dashboard on a laptop, with something in every page: `bun run demo [--reset] [--port 3030]`.
 * Starts a stand-in game for this host and for two remote controllers (scripts/demo/game.ts), seeds a
 * database on the first run (scripts/demo/seed.ts; --reset makes it again), runs an agent for each
 * remote controller, the dashboard itself with one-click sign-in per tier, and a trickle of live chat
 * through the ingest API. Everything is on 127.0.0.1 and stops with Ctrl+C. The data lives in
 * data/demo, apart from a real dashboard's data/.
 */
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { startDemoGame } from "./demo/game";

const { values: args } = parseArgs({
  options: {
    port: { type: "string", default: "3030" },
    reset: { type: "boolean", default: false },
  },
  strict: true,
});

const web = resolve(import.meta.dir, "..");
const dataDir = join(web, "data", "demo");
const port = Number(args.port);
const base = `http://localhost:${port}`;
// Made up for the demo, never anything real: every listener is on loopback
const ADMIN_KEY = "demo-admin-api-key";
const OPERATOR_SECRET = "demo-operator-secret";
const INGEST_KEY = "demo-ingest-key-demo-ingest-key-0000";

if (args.reset) rmSync(dataDir, { recursive: true, force: true });
mkdirSync(dataDir, { recursive: true });

const games = [
  startDemoGame({ port: 0, name: "Dainty Friends", mode: "PRIVATE", seed: 1, players: 40 }),
  startDemoGame({ port: 0, name: "Frontier", mode: "PUBLIC", seed: 2, players: 60 }),
  startDemoGame({ port: 0, name: "Aurora", mode: "PUBLIC", seed: 3, players: 24 }),
];
const [local, frontier, aurora] = games as [
  (typeof games)[0],
  (typeof games)[0],
  (typeof games)[0],
];

const env: Record<string, string> = {
  ...(process.env as Record<string, string>),
  NODE_ENV: "development",
  DASHBOARD_BASE_URL: base,
  DASHBOARD_DATA_DIR: dataDir,
  DASHBOARD_DEV_LOGIN: "on",
  DASHBOARD_OWNER_DISCORD_IDS: "100000000000000001",
  DASHBOARD_ADMIN_API_URL: `${local.url}/graphql`,
  DASHBOARD_ADMIN_API_KEY: ADMIN_KEY,
  DASHBOARD_BACKEND_URL: local.url,
  DASHBOARD_MANAGER_OPERATOR_SECRET: OPERATOR_SECRET,
  DASHBOARD_INGEST_KEY: INGEST_KEY,
  // vinext allows one dev server per directory; the demo is a second one beside `bun run dev`
  VINEXT_NO_DEV_LOCK: "1",
};

const children: ReturnType<typeof Bun.spawn>[] = [];
function stop(code = 0): never {
  for (const child of children) child.kill();
  for (const game of games) game.stop();
  process.exit(code);
}
process.on("SIGINT", () => stop());
process.on("SIGTERM", () => stop());

const codesFile = join(dataDir, "pairing-codes.json");
if (!existsSync(join(dataDir, "dashboard.db"))) {
  console.log("demo: seeding data/demo");
  const seed = Bun.spawnSync([process.execPath, "scripts/demo/seed.ts"], {
    cwd: web,
    env,
    stderr: "inherit",
  });
  if (seed.exitCode !== 0) stop(seed.exitCode ?? 1);
  const out = seed.stdout.toString().trim().split("\n").at(-1) ?? "{}";
  writeFileSync(codesFile, out);
}

children.push(
  Bun.spawn([process.execPath, "run", "dev", "--port", String(port), "--strictPort"], {
    cwd: web,
    env,
    stdout: "inherit",
    stderr: "inherit",
  }),
);

async function ready(): Promise<void> {
  for (let i = 0; i < 120; i++) {
    try {
      if ((await fetch(`${base}/api/health`)).status < 500) return;
    } catch {
      // not up yet
    }
    await Bun.sleep(1000);
  }
  console.error("demo: the dashboard did not start");
  stop(1);
}
await ready();

// The agents dial in once the dashboard answers; each pairs once, then keeps its token
const codes = JSON.parse(readFileSync(codesFile, "utf8")) as Record<string, string>;
for (const [name, game] of [
  ["frontier", frontier],
  ["aurora", aurora],
] as const) {
  children.push(
    Bun.spawn([process.execPath, "src/main.ts"], {
      cwd: join(web, "..", "agent"),
      env: {
        ...(process.env as Record<string, string>),
        DAINTY_AGENT_DASHBOARD_URL: base,
        DAINTY_AGENT_TOKEN_FILE: join(dataDir, `${name}-agent-token`),
        ...(existsSync(join(dataDir, `${name}-agent-token`))
          ? {}
          : { DAINTY_AGENT_PAIRING_CODE: codes[name] ?? "" }),
        DAINTY_AGENT_ADMIN_API_URL: `${game.url}/graphql`,
        DAINTY_AGENT_ADMIN_API_KEY: ADMIN_KEY,
        DAINTY_AGENT_BACKEND_URL: game.url,
        DAINTY_AGENT_OPERATOR_SECRET: OPERATOR_SECRET,
      },
      stdout: "inherit",
      stderr: "inherit",
    }),
  );
}

// Live chat, so the chat page and the overview move while you watch
const SPEAKERS = [
  ["11000", "Slayer_42"],
  ["11002", "Pangar_Pal"],
  ["11005", "Ramsgate_Rose"],
  ["11008", "Quillshot"],
];
const SAID = [
  "anyone for a quick hunt?",
  "gg, nice dodge",
  "darn, so close",
  "selling gold, cheap",
  "lfg heroic escalation",
  "noob question: where is the forge?",
];
let said = 0;
setInterval(() => {
  const [userId, name] = SPEAKERS[said % SPEAKERS.length]!;
  void fetch(`${base}/api/ingest/chat`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${INGEST_KEY}` },
    body: JSON.stringify({
      messages: [
        {
          id: `live-${Date.now()}`,
          at: Date.now(),
          channel: "city",
          roomId: "room-0",
          sender: { userId, name },
          body: SAID[said++ % SAID.length],
        },
      ],
    }),
  }).catch(() => {});
}, 8000);

console.log(`\ndemo: ${base}/login (sign in as any tier; Rook is the seeded owner)\n`);
