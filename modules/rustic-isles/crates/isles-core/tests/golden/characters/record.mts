// Records the golden answers of the characters-loadouts routes from the Node reference (RU5): the real app on its
// SQLite provider, driven through scenario.json over HTTP. Run by record.sh inside a node:22 container, from a copy of
// reference/<version>/ParadoxBackend with its dependencies installed:
//   node --import tsx record.mts <scenario.json> <out.json>
// The Rust side (tests/characters_golden.rs) replays the same scenario against its handlers and compares.
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";

const [scenarioPath, outPath] = process.argv.slice(2);
const { publicKey, privateKey } = crypto.generateKeyPairSync("rsa", { modulusLength: 2048 });
process.env.AUTH_SIGNING_PRIVKEY_B64 = Buffer.from(privateKey.export({ type: "pkcs8", format: "pem" })).toString("base64");
process.env.AUTH_SIGNING_PUBKEY_B64 = Buffer.from(publicKey.export({ type: "spki", format: "pem" })).toString("base64");
Object.assign(process.env, {
    NODE_ENV: "production",
    PERSISTENCE_PROVIDER: "sqlite",
    MYSTICPARADOX_SQLITE_PATH: ":memory:",
    MYSTICPARADOX_RATE_LIMIT: "off",
    API_KEY_HASH_SECRET: "x".repeat(40),
    GAMESERVER_API_KEYS: "gs-key",
    TARGET_CHANGELIST: process.env.TARGET_CHANGELIST ?? "647472",
});
delete process.env.GAMESERVER_NO_PLAYER_USER_ID;
delete process.env.DEV_USER_ID;

const quiet = (..._args: unknown[]) => undefined;
console.log = quiet; console.info = quiet; console.warn = quiet; console.debug = quiet;

const { app } = await import("./src/app");
const { GetRepositories } = await import("./src/persistence");
const { DrainAndRegisterAPIKeys } = await import("./src/controllers/apikeys");
const { SignMetagameJWTForUid } = await import("./src/controllers/auth");

await DrainAndRegisterAPIKeys();
const server = http.createServer(app as any);
await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
const port = (server.address() as any).port;

const scenario = JSON.parse(fs.readFileSync(scenarioPath, "utf8"));
const saved: Record<string, string> = {};
const out: any[] = [];

function pick(value: any, path: string): string {
    return String(path.split(".").reduce((v, k) => v?.[k], value));
}

for (const step of scenario.steps) {
    if (step.setEnv) {
        for (const [k, v] of Object.entries(step.setEnv)) process.env[k] = String(v);
        continue;
    }
    if (step.setup === "journey") {
        const journeys: any = GetRepositories().playerJourney;
        const record = { userId: step.user, nodes: JSON.stringify(step.nodes), updateVersion: 1, revision: 0 };
        await journeys.createIfAbsent(record);
        continue;
    }
    const headers: Record<string, string> = {};
    const [kind, who] = String(step.as).split(":");
    if (kind === "gameserver") headers["x-mysticparadox-gameserver-apikey"] = "gs-key";
    if (who) headers.authorization = `Bearer ${SignMetagameJWTForUid(who)}`;
    let body: string | undefined;
    if (step.rawBody !== undefined) body = step.rawBody;
    else if (step.body !== undefined) body = JSON.stringify(step.body);
    if (body !== undefined) headers["content-type"] = step.contentType ?? "application/json";
    const path = String(step.path).replace(/\{(\w+)\}/g, (_, name) => saved[name] ?? "MISSING");
    const payload = body === undefined ? undefined : body.replace(/\{(\w+)\}/g, (_, name) => saved[name] ?? "{" + name + "}");
    const response = await fetch(`http://127.0.0.1:${port}${path}`, { method: step.method, headers, body: payload });
    const text = await response.text();
    out.push({
        name: step.name,
        status: response.status,
        contentType: response.headers.get("content-type"),
        body: text,
    });
    for (const [name, p] of Object.entries<string>(step.save ?? {})) saved[name] = pick(JSON.parse(text), p);
}

fs.writeFileSync(outPath, JSON.stringify({ node: process.version, steps: out }, null, 1) + "\n");
server.close();
process.exit(0);
