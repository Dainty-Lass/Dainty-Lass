/*
 * Records what Rustic Isles' Node backend (reference/1.14.7/ParadoxBackend) answers to scenarios.json, for the Rust
 * golden test (tests/inventory_golden.rs): the real inventory and progression routers and controllers over the real
 * SQLite persistence provider, in memory. Only three things are replaced: the authentication middleware (the scenario
 * names the AuthData instead of a token or API key), the starter bootstrap (the characters area's) and the game data
 * folder (game-data/ here). Run through record.sh, in a node:22 container.
 *
 *   docker run --rm -v "$PWD:/repo:ro" -v "$PWD/out:/out" node:22 sh /repo/crates/isles-core/tests/golden/inventory/record.sh
 */
"use strict";
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const Module = require("node:module");

const BACKEND = process.cwd();
process.env.NODE_ENV = "production"; // Express answers errors with the production page
process.env.PERSISTENCE_PROVIDER = "sqlite";
process.env.RUSTIC_ISLES_SQLITE_PATH = ":memory:";
process.env.PARADOX_GAME_DATA_DIR = path.join(BACKEND, "game-data-golden");
process.env.GAMESERVER_NO_PLAYER_USER_ID = "u1";
delete process.env.DEV_USER_ID;
delete process.env.RUSTIC_ISLES_REFORGE;
delete process.env.KEEP_WEAPON_ITEM_LEVELS;

const { seed, scenarios } = JSON.parse(fs.readFileSync(path.join(BACKEND, "scenarios.json"), "utf8"));

// The scenario's AuthData stands in for HasParadoxBackendAuth.
const originalLoad = Module._load;
Module._load = function (request, parent, isMain) {
    if (/middleware\/HasParadoxBackendAuth$/.test(request)) {
        return {
            HasParadoxBackendAuth(req, res, next) {
                const header = req.headers["x-test-auth"];
                if (header === undefined) {
                    res.status(401);
                    res.send();
                    return;
                }
                req.AuthData = JSON.parse(header);
                next();
            },
            IsCurrentLauncherAccountEligible: async () => true,
        };
    }
    const loaded = originalLoad.apply(this, arguments);
    // The starter bootstrap belongs to the characters area: an existing inventory is served as it is
    if (/\/starterManifest$/.test(request)) {
        return {
            ...loaded,
            EnsureStarterBootstrapRecords: async () => undefined,
            EnsureStarterBootstrapRecordsInTransaction: async (repos, _userId, characterId, session) => ({
                inventory: await repos.inventories.findByCharacterId(characterId, session),
            }),
        };
    }
    return loaded;
};

function normalize(text) {
    return text
        .replace(/\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z/g, "<TS>")
        .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, "<UUID>")
        .replace(/[A-Z2-7]{26}/g, "<ID>");
}

async function main() {
    const express = require("express");
    const { GetRepositories } = require(path.join(BACKEND, "src/persistence"));
    const { inventoryRouter } = require(path.join(BACKEND, "src/routes/inventory"));
    const { progressionRouter } = require(path.join(BACKEND, "src/routes/progression"));

    const repos = GetRepositories();
    for (const user of seed.users) {
        for (const character of user.characters) {
            await repos.characters.create({
                characterId: character.characterId,
                userId: user.userId,
                createdDate: "2026-10-01T00:00:00.000Z",
                lastModifiedDate: "2026-10-01T00:00:00.000Z",
                name: `Hunter ${character.characterId}`,
                updateVersion: 1,
                data: "{}",
            });
            await repos.inventories.create({
                characterId: character.characterId,
                userId: user.userId,
                instancedItems: JSON.stringify(character.instanced),
                stackedItems: JSON.stringify(character.stacked),
                revision: 0,
                bootstrapVersion: "golden",
            });
        }
        await repos.wallets.createIfMissing({ userId: user.userId, balances: { ...user.wallet } });
        for (const [track, progress] of Object.entries(user.tracks ?? {})) {
            await repos.progressionTracks.increment(user.userId, track, progress);
        }
    }

    const app = express();
    app.use(express.json());
    app.use(express.urlencoded({ extended: true }));
    app.use("/", inventoryRouter);
    app.use("/", progressionRouter);
    app.use((req, res) => {
        res.status(404);
        res.send();
    });
    const server = http.createServer(app);
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    const base = `http://127.0.0.1:${server.address().port}`;

    const recorded = [];
    for (const scenario of scenarios) {
        const headers = {};
        let body;
        if (scenario.auth !== undefined) headers["x-test-auth"] = JSON.stringify(scenario.auth);
        if (scenario.raw !== undefined || scenario.body !== undefined) {
            headers["content-type"] = scenario.contentType ?? "application/json";
            body = scenario.raw !== undefined ? scenario.raw : JSON.stringify(scenario.body);
        }
        const response = await fetch(base + scenario.path, { method: scenario.method, headers, body });
        const text = await response.text();
        recorded.push({
            name: scenario.name,
            status: response.status,
            contentType: response.headers.get("content-type"),
            body: normalize(text),
        });
    }
    server.close();
    fs.writeFileSync(process.argv[2] ?? "expected.json", JSON.stringify(recorded, null, 1) + "\n");
    process.exit(0);
}

main().catch((error) => {
    console.error(error);
    process.exit(1);
});
