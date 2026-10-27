// RU5 game-data golden capture: runs the reference handlers of routes/system.ts and routes/tuning.ts, unchanged, under
// Node 22 and records what each writes (status, content type, body text) for the cases below. The imports a handler
// does not reach (Mongo repositories, controllers) are stubbed; the router, the logger and the auth middleware are
// replaced by recorders; Date.now is pinned. The output, game_data.json, is what the Rust handlers are compared with.
//
//   docker run --rm -v "$PWD/../../../..":/repo -w /repo node:22 \
//     node crates/isles-core/tests/golden/capture.mjs reference crates/isles-core/tests/golden
//
// The game data it reads is progression_config.synthetic.json beside this file, never a game export.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

const [referenceRoot, goldenDir] = process.argv.slice(2);
const NOW = Date.parse("2026-10-26T12:00:00.000Z");
Date.now = () => NOW;

const VERSIONS = ["1.14.7", "1.12.0"];
const GAME_DATA = path.resolve(goldenDir);
const ORIGIN = "gauntlet-prod.steelyard.ca";
const PASSES = "progression_config.synthetic.json";

// name, version filter (undefined = both), request. `origin` is the /__origin host the runtime prefixes.
const ONLY_1147 = ["1.14.7"];
const CASES = [
    ["heartbeat", { method: "POST", path: "/heartbeat" }],
    ["event", { method: "POST", path: "/event" }],
    ["crash ping", { method: "GET", path: "/crash/CrashReporter/Ping" }],
    ["crash check report", { method: "POST", path: "/crash/CrashReporter/CheckReport" }],
    ["survey config", { method: "GET", path: "/survey/config" }],
    ["account migrate", { method: "POST", path: "/account/migrate" }],
    ["profile update", { method: "POST", path: "/profile/update" }],
    ["trials all, no body", { method: "POST", path: "/trials/leaderboards/all" }],
    ["trials all, defaults", { method: "POST", path: "/trials/leaderboards/all", json: "{}" }],
    ["trials all, echoed fields", { method: "POST", path: "/trials/leaderboards/all", json: '{"difficulty":3.0,"page":2,"page_size":"50","trial_id":"T_001","extra":1}' }],
    ["trials all, nulls and objects", { method: "POST", path: "/trials/leaderboards/all", json: '{"difficulty":null,"page":{"a":[1,2e0]},"trial_id":null}' }],
    ["trials all, array body", { method: "POST", path: "/trials/leaderboards/all", json: '[{"difficulty":9}]' }],
    ["trials all, form body", { method: "POST", path: "/trials/leaderboards/all", form: "difficulty=2&trial_id=a%20b" }],
    ["trials individual", { method: "POST", path: "/trials/leaderboards/solo/individual" }],
    ["motd", { method: "POST", path: "/motd/" }],
    ["motd without slash", { method: "POST", path: "/motd" }],
    ["motd trigger", { method: "GET", path: "/motd/trigger?event_name=test" }],
    ["playertreatments", { method: "GET", path: "/playertreatments" }],
    ["playertreatments slash", { method: "GET", path: "/playertreatments/" }],
    ["playertreatments user", { method: "GET", path: "/playertreatments/some-user" }],
    ["playertreatments legacy off", { method: "GET", path: "/playertreatments", env: { COHORT_TREATMENT_LEGACY: "0" } }],
    ["playertreatments legacy blank", { method: "GET", path: "/playertreatments", env: { COHORT_TREATMENT_LEGACY: "" } }],
    ["eventstats", { method: "GET", path: "/eventstats/" }],
    ["progression config", { method: "GET", path: "/progression/config" }],
    ["progression config, widening off", { method: "GET", path: "/progression/config", env: { GAMESERVER_WIDEN_HUNTPASS_WINDOWS: "0" } }],
    ["progression config, served pass", { method: "GET", path: "/progression/config", env: { GAMESERVER_HUNTPASS_ID: " season19 " } }],
    ["progression config, recent seasons", { method: "GET", path: "/progression/config", env: { GAMESERVER_RECENT_SEASONS: "season60, ,season61,season60,season43" } }],
    ["progression config, no recent seasons", { method: "GET", path: "/progression/config", env: { GAMESERVER_RECENT_SEASONS: "" } }],
    ["gauntlet config", { method: "GET", path: "/config", origin: ORIGIN }],
    ["gauntlet config probe off", { method: "GET", path: "/config", origin: ORIGIN, env: { GAUNTLET_CONFIG_PROBE: "0" } }],
    ["config on another origin", { method: "GET", path: "/config", origin: "mailbox-prod.steelyard.ca" }],
    ["gauntlet progression", { method: "GET", path: "/progression/ESC_SEASON_1/user-1", origin: ORIGIN }, ONLY_1147],
    ["gauntlet progression, empty season", { method: "GET", path: "/progression//INVALID", origin: ORIGIN }, ONLY_1147],
    ["gauntlet level access", { method: "GET", path: "/progression/level_access/user-1?account_id=x", origin: ORIGIN }, ONLY_1147],
    ["gauntlet level access, upper case", { method: "GET", path: "/PROGRESSION/LEVEL_ACCESS/user-1", origin: ORIGIN }, ONLY_1147],
    ["gauntlet personal rewards", { method: "GET", path: "/rewards/personal/ESC_SEASON_1/user-1", origin: ORIGIN }],
    ["personal rewards on another origin", { method: "GET", path: "/rewards/personal/ESC_SEASON_1/user-1", origin: "mailbox-prod.steelyard.ca" }],
    ["gauntlet guild rewards", { method: "GET", path: "/rewards/guild/ESC_SEASON_1/user-1", origin: ORIGIN }],
    ["gauntlet leaderboard", { method: "GET", path: "/leaderboard/get_leaderboard/ESC_SEASON_1/guild", origin: ORIGIN }],
    ["mailbox", { method: "GET", path: "/all/" }],
    ["patchnotes", { method: "GET", path: "/patchnotes/en/1.14.7.123" }],
    ["patchnotes, encoded language", { method: "GET", path: "/patchnotes/pt%2DBR/1.0" }],
    ["seasonal event schedule", { method: "GET", path: "/game_tuning/seasonal_event_schedule" }],
    ["huntpass xp config", { method: "GET", path: "/game_tuning/huntpass_xp_config" }],
    ["island content config", { method: "GET", path: "/game_tuning/island_content_config" }],
    ["bounty game data", { method: "GET", path: "/game_tuning/bounty_game_data" }],
    ["bounty game data daily", { method: "GET", path: "/game_tuning/bounty_game_data_daily" }],
    ["bounty game data weekly", { method: "GET", path: "/game_tuning/bounty_game_data_weekly" }],
];

// ---- the reference code, imports replaced ---------------------------------------------------------------------------

const registered = [];
class RouterStub {
    constructor() { this.owner = registered; }
}
for (const method of ["get", "post", "put", "delete", "patch"]) {
    RouterStub.prototype[method] = function (paths, ...handlers) {
        this.owner.push({ method: method.toUpperCase(), paths: Array.isArray(paths) ? paths : [paths], handlers });
        return this;
    };
}
RouterStub.prototype.use = function () { return this; };

const stub = () => new Proxy(function () {}, {
    get: (_t, key) => (key === Symbol.toPrimitive ? () => "" : stub()),
    apply: () => stub(),
    construct: () => stub(),
});

function authMarker(_req, _res, next) { next(); }

function importedNames(line) {
    const match = /^import\s+(?:type\s+)?(?:\{([^}]*)\}|(\w+))\s+from/.exec(line);
    if (!match) return [];
    return (match[1] ?? match[2]).split(",").map((part) => part.trim().replace(/^type\s+/, "").split(/\s+as\s+/).pop()).filter(Boolean);
}

async function load(version, file) {
    const src = path.join(referenceRoot, version, "ParadoxBackend", "src");
    const work = fs.mkdtempSync(path.join(os.tmpdir(), `golden-${version}-`));
    for (const name of ["gauntletPayloads", "seasonWindow"]) {
        const from = path.join(src, `${name}.ts`);
        if (!fs.existsSync(from)) continue;
        fs.writeFileSync(path.join(work, `${name}.ts`), fs.readFileSync(from, "utf8").replace(/from "\.\/seasonWindow"/g, 'from "./seasonWindow.ts"'));
    }
    const progression = fs.readFileSync(path.join(src, "controllers", "progression.ts"), "utf8");
    const activeId = /ACTIVE_HUNTPASS_PROGRESSION_ID\s*=\s*"([^"]+)"/.exec(progression)?.[1] ?? "season19";
    const hasGauntlet = fs.existsSync(path.join(work, "gauntletPayloads.ts"));
    const provided = {
        Router: "const Router = () => new __RouterStub();",
        logger: "const logger = { info() {}, debug() {}, warn() {}, error() {} };",
        HasParadoxBackendAuth: "const HasParadoxBackendAuth = __auth;",
        loadGameData: `const loadGameData = (name) => JSON.parse(__fs.readFileSync(__path.join(${JSON.stringify(GAME_DATA)}, name === "progression_config.json" ? ${JSON.stringify(PASSES)} : name), "utf8"));`,
        ACTIVE_HUNTPASS_PROGRESSION_ID: `const ACTIVE_HUNTPASS_PROGRESSION_ID = ${JSON.stringify(activeId)};`,
        GetEscalationTuning: 'const GetEscalationTuning = () => ({ status: "missing" });',
    };
    const real = hasGauntlet
        ? 'import { GauntletConfigPayload, GauntletGuildRewardsPayload, GauntletLeaderboardPayload, GauntletProgressionPayload } from "./gauntletPayloads.ts";\nimport { ActiveSeasonWindow } from "./seasonWindow.ts";\n'
        : "";
    const realNames = hasGauntlet ? new Set(["GauntletConfigPayload", "GauntletGuildRewardsPayload", "GauntletLeaderboardPayload", "GauntletProgressionPayload", "ActiveSeasonWindow"]) : new Set();
    const lines = fs.readFileSync(path.join(src, "routes", `${file}.ts`), "utf8").split(/\r?\n/);
    const names = new Set();
    const body = lines.map((line) => {
        if (!/^import\s/.test(line)) return line;
        importedNames(line).forEach((name) => names.add(name));
        return "";
    }).join("\n");
    const defs = [...names].map((name) => provided[name] ?? (realNames.has(name) ? "" : `const ${name} = __stub();`)).join("\n");
    const target = path.join(work, `${file}.ts`);
    fs.writeFileSync(target, `${real}${defs}\n${body}`);
    globalThis.__RouterStub = RouterStub;
    globalThis.__auth = authMarker;
    globalThis.__stub = stub;
    globalThis.__fs = fs;
    globalThis.__path = path;
    await import(pathToFileURL(target).href);
}

// ---- running a request through the registered routes, as Express orders them -------------------------------------------

function matches(spec, pathname) {
    if (spec instanceof RegExp) return spec.exec(pathname);
    const want = spec.split("/");
    const got = pathname.replace(/(?<=.)\/$/, "").split("/");
    const wantTrimmed = spec.length > 1 && spec.endsWith("/") ? spec.slice(0, -1).split("/") : want;
    if (wantTrimmed.length !== got.length) return null;
    const params = {};
    for (let i = 0; i < got.length; i++) {
        if (wantTrimmed[i].startsWith(":")) {
            if (got[i] === "") return null;
            params[wantTrimmed[i].slice(1)] = decodeURIComponent(got[i]);
        } else if (wantTrimmed[i].toLowerCase() !== got[i].toLowerCase()) return null;
    }
    return { params };
}

function run(routes, request) {
    const url = new URL(request.path, "http://x");
    const result = { status: 404, contentType: null, body: "" };
    let finished = false;
    const res = {
        status(code) { result.status = code; return this; },
        type(kind) { result.contentType = kind === "text/plain" ? "text/plain; charset=utf-8" : kind; return this; },
        json(value) { result.contentType = "application/json; charset=utf-8"; result.body = JSON.stringify(value); finished = true; return this; },
        send(value) { if (typeof value === "string") { result.contentType ??= "text/html; charset=utf-8"; result.body = value; } finished = true; return this; },
        end() { finished = true; return this; },
    };
    for (const route of routes) {
        if (route.method !== request.method) continue;
        let params = null;
        for (const spec of route.paths) {
            const hit = matches(spec, url.pathname);
            if (hit) {
                params = spec instanceof RegExp ? Object.fromEntries([...hit].slice(1).map((v, i) => [i, decodeURIComponent(v)])) : hit.params;
                break;
            }
        }
        if (params === null) continue;
        const req = { method: request.method, params, originHost: request.origin, body: request.parsedBody, headers: {}, url: request.path };
        let advanced = true;
        result.status = 200;
        for (const handler of route.handlers) {
            if (!advanced) break;
            advanced = false;
            handler(req, res, () => { advanced = true; });
            if (finished) return result;
        }
        if (finished) return result;
        result.status = 404;
    }
    return result;
}

function parsedBody(request) {
    if (request.json !== undefined) return JSON.parse(request.json);
    if (request.form !== undefined) return Object.fromEntries(new URLSearchParams(request.form));
    return undefined;
}

const cases = [];
for (const version of VERSIONS) {
    registered.length = 0;
    await load(version, "system");
    await load(version, "tuning");
    const routes = [...registered];
    for (const [name, definition, only] of CASES) {
        if (only && !only.includes(version)) continue;
        const env = definition.env ?? {};
        Object.assign(process.env, env);
        const request = { ...definition, parsedBody: parsedBody(definition) };
        const result = run(routes, request);
        for (const key of Object.keys(env)) delete process.env[key];
        cases.push({
            name: `${version}: ${name}`,
            version,
            request: {
                method: definition.method,
                path: definition.path,
                origin: definition.origin ?? null,
                json: definition.json ?? null,
                form: definition.form ?? null,
                env: definition.env ?? {},
            },
            expect: { status: result.status, content_type: result.contentType, body: result.body },
        });
    }
}

fs.writeFileSync(path.join(goldenDir, "game_data.json"), JSON.stringify({ now: NOW, cases }, null, 1) + "\n");
console.log(`${cases.length} cases`);
