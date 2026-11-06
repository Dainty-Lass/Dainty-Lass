// Records what Rustic Isles' own matchmaking and party routers answer (RU5 matchmaking-parties golden tests).
//
//   docker run --rm -v <repo>/reference/1.14.7/ParadoxBackend:/ref:ro -v <repo>/crates/isles-mit-parity/tests/golden:/out \
//     node:22 sh -c 'cp -r /ref /work && cd /work && npm ci --ignore-scripts && node /out/capture.cjs /out/matchmaking-parties.json'
//
// The real routers (src/routes/matchmaking.ts, party.ts) and controllers run on the SQLite provider, with a fake
// DeployServer and HasParadoxBackendAuth replaced by "the x-test-user header is the userId". The scenario below is
// replayed by tests/matchmaking_routes.rs against the Rust handlers, which must answer alike (random ids aside).
"use strict";
const fs = require("node:fs");
const http = require("node:http");
const Module = require("node:module");

const out = process.argv[2];
const NODE_ENV = "production";
const deploy = http.createServer((req, res) => {
    res.setHeader("content-type", "application/json");
    if (req.method === "POST" && req.url === "/api/matchmaker/handle-matchmaking-for-player") {
        req.resume();
        req.on("end", () => res.end(JSON.stringify({ host: "203.0.113.10", port: 7777 })));
    } else if (req.url === "/api/hub-status") {
        res.end(JSON.stringify({ ramsgate: { ready: true, host: "203.0.113.10", port: 7777, processId: 1 } }));
    } else {
        res.statusCode = 404;
        res.end("{}");
    }
});

deploy.listen(0, "127.0.0.1", async () => {
    Object.assign(process.env, {
        NODE_ENV,
        MATCHMAKING_MODE: "DEPLOYSERVER",
        DEPLOYSERVER_URL: `127.0.0.1:${deploy.address().port}`,
        HUNT_GO_NOW_MIN_DELAY_MS: "40",
        HUNT_GO_NOW_MAX_DELAY_MS: "40",
        HUB_GO_NOW_MIN_DELAY_MS: "40",
        HUB_GO_NOW_MAX_DELAY_MS: "40",
        GO_NOW_LOAD_PADDING_PER_JOINER_MS: "1",
        QOS_TARGET_URL: "qos.example.net",
        RUSTIC_ISLES_P2P_MODULE: "off",
        PERSISTENCE_PROVIDER: "sqlite",
        RUSTIC_ISLES_SQLITE_PATH: "/tmp/golden.sqlite",
        TARGET_CHANGELIST: "647472"
    });
    const original = Module._load;
    Module._load = function (request, ...rest) {
        if (/middleware\/HasParadoxBackendAuth$/.test(request)) {
            return {
                HasParadoxBackendAuth: (req, res, next) => {
                    const user = req.headers["x-test-user"];
                    if (!user) { res.status(401); res.send(); return; }
                    req.AuthData = { userId: user };
                    next();
                }
            };
        }
        return original.call(this, request, ...rest);
    };
    // resolved from the copy of the reference, where its dependencies are installed
    const work = Module.createRequire("/work/package.json");
    work("tsx/cjs");
    const express = work("express");
    const { matchmakingRouter } = work("./src/routes/matchmaking.ts");
    const { partyRouter } = work("./src/routes/party.ts");
    const app = express();
    app.use(express.json());
    app.use("/", matchmakingRouter);
    app.use("/", partyRouter);
    const server = app.listen(0, "127.0.0.1");
    await new Promise((resolve) => server.once("listening", resolve));
    const base = `http://127.0.0.1:${server.address().port}`;

    const steps = JSON.parse(fs.readFileSync(`${__dirname}/scenario.json`, "utf8"));
    const responses = {};
    const record = [];
    for (const step of steps) {
        if (step.sleep) { await new Promise((r) => setTimeout(r, step.sleep)); continue; }
        // "$step.field" in a body is a field of an earlier answer
        const fill = (value) => {
            if (typeof value === "string" && value.startsWith("$")) {
                const [name, field] = value.slice(1).split(".");
                return responses[name][field];
            }
            return value && typeof value === "object" ? Object.fromEntries(Object.entries(value).map(([k, v]) => [k, fill(v)])) : value;
        };
        const body = fill(step.body);
        const headers = {};
        if (step.user) headers["x-test-user"] = step.user;
        let payload;
        if (step.raw !== undefined) { payload = step.raw; headers["content-type"] = "application/json"; }
        else if (body !== undefined) { payload = JSON.stringify(body); headers["content-type"] = "application/json"; }
        const response = await fetch(base + step.path, { method: step.method, headers, body: payload });
        const text = await response.text();
        responses[step.name] = (() => { try { return JSON.parse(text); } catch { return undefined; } })();
        record.push({
            name: step.name,
            method: step.method,
            path: step.path,
            status: response.status,
            contentType: response.headers.get("content-type"),
            partyRevision: response.headers.get("x-mystic-party-revision"),
            body: text
        });
    }
    fs.writeFileSync(out, JSON.stringify(record, null, 2) + "\n");
    console.log(`recorded ${record.length} answers`);
    server.close();
    deploy.close();
    process.exit(0);
});
