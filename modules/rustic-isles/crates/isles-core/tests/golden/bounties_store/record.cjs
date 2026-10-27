// Records what Node's bounty, cooldown, entitlement, wallet and store routes answer (RU5 bounties-store golden tests).
// Run inside a copy of reference/<version>/ParadoxBackend after `npm ci`, as `node --require tsx/cjs record.cjs`; see
// run.sh. The routers are Node's own (routes/system.ts and routes/store.ts, with the controllers under them). Replaced:
// the logger, the auth middleware (an `x-test-auth` header: gs, player:<id>), the repositories (in memory, as the
// contracts describe them), the purchase tokens, the character lookup and RunInventoryTransaction. The clock is fixed.
//
// Prints { version, steps: [{ name, method, path, auth, body, status, contentType, text }] }. `{token}` in a path is the
// purchase token the last /token answered; a token itself is never compared (shape only).
const http = require("node:http");
const path = require("node:path");

const Version = process.argv[2] ?? "1.14.7";
const NOW = Date.UTC(2026, 9, 26, 12, 0, 0);
const RealDate = Date;
global.Date = class extends RealDate {
    constructor(...args) {
        if (args.length === 0) super(NOW);
        else super(...args);
    }
    static now() {
        return NOW;
    }
};

process.env.PARADOX_GAME_DATA_DIR = "/golden/game-data";
process.env.MYSTICPARADOX_P2P_MODULE = "off";

const Quiet = () => undefined;
function Fake(modulePath, exports) {
    const Resolved = require.resolve(modulePath);
    require.cache[Resolved] = { id: Resolved, filename: Resolved, loaded: true, exports, children: [], paths: [] };
}
const Src = path.join(process.cwd(), "src");
Fake(path.join(Src, "logger"), { logger: { level: "silent", trace: Quiet, debug: Quiet, info: Quiet, warn: Quiet, error: Quiet, fatal: Quiet, child: () => ({}) }, ApplyDiagnosticsLogProfile: Quiet });
Fake(path.join(Src, "middleware/HasParadoxBackendAuth"), {
    HasParadoxBackendAuth: (req, res, next) => {
        const Header = req.headers["x-test-auth"];
        if (Header === undefined) { res.status(401); res.send(); return; }
        req.AuthData = Header === "gs" ? { IsGameserver: true } : { userId: Header.slice("player:".length) };
        next();
    },
});

// ---- in-memory repositories
const Accounts = new Map([["p1", { userId: "p1", name: "p1", notes: 0 }], ["p2", { userId: "p2", name: "p2", notes: 0 }]]);
const Bounties = new Map();
const Cooldowns = new Map();
const Wallets = new Map();
const Entitlements = new Map();
const Ledger = [];
const Repos = {
    accounts: {
        async findByUserId(id) { return Accounts.get(id); },
        async create(a) { Accounts.set(a.userId, a); },
    },
    bountyStates: {
        async get(id) { return Bounties.get(id); },
        async saveIfVersion(record, expected) {
            const Existing = Bounties.get(record.userId);
            if (expected === undefined ? Existing !== undefined : Existing?.updateVersion !== expected) return undefined;
            Bounties.set(record.userId, structuredClone(record));
            return record;
        },
    },
    cooldownStates: {
        async get(id) { return Cooldowns.get(id); },
        async saveIfVersion(record, expected) {
            const Existing = Cooldowns.get(record.userId);
            if (expected === undefined ? Existing !== undefined : Existing?.updateVersion !== expected) return undefined;
            Cooldowns.set(record.userId, structuredClone(record));
            return record;
        },
    },
    wallets: {
        async findByUserId(id) { return Wallets.get(id); },
        async createIfMissing(wallet) {
            if (!Wallets.has(wallet.userId)) Wallets.set(wallet.userId, structuredClone(wallet));
            return Wallets.get(wallet.userId);
        },
        async incrementBalance(id, catalogId, delta) {
            const Wallet = Wallets.get(id);
            if (Wallet === undefined) return undefined;
            if (delta < 0 && (Wallet.balances[catalogId] ?? -Infinity) < -delta) return undefined;
            Wallet.balances[catalogId] = (Wallet.balances[catalogId] ?? 0) + delta;
            return Wallet;
        },
    },
    entitlements: {
        async findByUserId(id) { return Entitlements.get(id); },
        async grantIfMissing(id, name, duration, activatedDate, sourceSkuId) {
            const Record = Entitlements.get(id) ?? { userId: id, entitlements: [] };
            Entitlements.set(id, Record);
            if (Record.entitlements.some((e) => e.name === name)) return { granted: false };
            Record.entitlements.push({ name, duration, activatedDate, sourceSkuId });
            return { granted: true };
        },
    },
};
Fake(path.join(Src, "persistence"), {
    GetRepositories: () => Repos,
    GetUnitOfWork: () => ({}),
    GetSqliteDirectAccess: () => ({
        async findInventoryLedgerClaim(userId, transactionId) {
            return Ledger.find((r) => r.userId === userId && r.transactionId === transactionId);
        },
    }),
});
Fake(path.join(Src, "persistence/mongo/client"), { GetMongoDb: async () => { throw new Error("no database"); } });

// ---- the other areas' seams
Fake(path.join(Src, "controllers/character"), { GetCharactersForUid: async (user) => (Accounts.has(user) ? [{ id: "char-1" }] : []) });
Fake(path.join(Src, "controllers/auth"), {
    SignStorePurchaseToken: (claims) => `n.${Buffer.from(JSON.stringify(claims)).toString("base64url")}`,
    ValidateStorePurchaseToken: (token) => {
        if (!token.startsWith("n.")) throw new Error("bad token");
        return JSON.parse(Buffer.from(token.slice(2), "base64url").toString());
    },
    SignLadyLuckPurchaseToken: (claims) => `n.${Buffer.from(JSON.stringify(claims)).toString("base64url")}`,
    ValidateLadyLuckPurchaseToken: (token) => {
        if (!token.startsWith("n.")) throw new Error("bad token");
        return JSON.parse(Buffer.from(token.slice(2), "base64url").toString());
    },
});
const Wallet = require(path.join(Src, "controllers/wallet"));
class Conflict extends Error {}
class Mismatch extends Error {}
// The ledger contract of RunInventoryTransaction, and a wallet that refuses a debit over 100 of anything.
Fake(path.join(Src, "controllers/inventory"), {
    InventoryTransactionConflictError: Conflict,
    InventoryTransactionMismatchError: Mismatch,
    RunInventoryTransaction: async (userId, characterId, transactionId, instancedAdd, stackedAdd, instancedRemove, stackedRemove) => {
        const Body = JSON.stringify([instancedAdd, stackedAdd, instancedRemove, stackedRemove]);
        const Existing = Ledger.find((r) => r.userId === userId && r.characterId === characterId && r.transactionId === transactionId);
        if (Existing) {
            if (Existing.body !== Body) throw new Mismatch(transactionId);
            return { replay: true };
        }
        for (const Item of stackedRemove) {
            if (Item.quantity > 100) throw new Wallet.InsufficientBalanceError(userId, Item.catalogId, -Item.quantity, 100);
        }
        Ledger.push({ userId, characterId, transactionId, body: Body });
        return { applied: true };
    },
});
// 1.14.7 only
if (require("node:fs").existsSync(path.join(Src, "activity"))) Fake(path.join(Src, "activity/activityMonitor"), { RecordChallengeClaimsActivity: Quiet });

const express = require("express");
const { systemRouter } = require(path.join(Src, "routes/system"));
const { storeRouter } = require(path.join(Src, "routes/store"));
const App = express();
App.use(express.json());
App.use(systemRouter);
App.use(storeRouter);
App.use((req, res) => { res.status(404); res.send(); });

// ---- the cases (the request list is the same for both versions; each answers as its own code does)
const SEASON = "43_1_1_Challenge_Season_A-season43-0";
const season = (progress, claimed) => ({ bounty_id: SEASON, premium_bounty: false, slot_index: 0, objectives: [{ objective_id: "Obj", progress }], drafted_timestamp: "2026-07-30T17:00:00.000Z", update_version: 0, claimed });
const SAVE = { bounties: [season(3, false)], draft_data_weekly: { current_draft_choices: [], previous_draft_selections: [], bronze_count: 0, silver_count: 0, gold_count: 1 } };
const BASE = { bounty_id: "Bounty_Bronze_X", drafted_timestamp: "2026-10-10T00:00:00.000Z", claimed: false, objectives: [{ objective_id: "A", progress: 1 }] };
const SPROUT = "58E91700491B00FA7DB373809E250E45";
const BATCH = { cooldowns: [{ cooldown_id: "bounty_daily", cooldown_started_date: "2026-10-09T17:00:00.000Z" }, { cooldown_id: "season43", cooldown_started_date: "2026-10-22T17:00:00Z" }] };
const P1 = "player:p1";
const P2 = "player:p2";
const J = JSON.stringify;
// [name, method, path, auth, body (object or raw string), credit] ; credit: a wallet top-up applied before the step
const STEPS = [
    ["bounty game data", "GET", "/bounty/game-data", "gs"],
    ["bounty game data, no credential", "GET", "/bounty/game-data"],
    ["bounty own, empty", "GET", "/bounty/p1", P1],
    ["bounty other player", "GET", "/bounty/p2", P1],
    ["bounty server reads any", "GET", "/bounty/p2", "gs"],
    ["bounty save by a player", "POST", "/bounty/p1", P1, SAVE],
    ["bounty save", "POST", "/bounty/p1", "gs", SAVE],
    ["bounty read back", "GET", "/bounty/p1", P1],
    ["bounty reseed at 0", "POST", "/bounty/p1", "gs", { bounties: [season(0, false)] }],
    ["bounty claim", "POST", "/bounty/p1", "gs", { bounties: [season(5, true)] }],
    ["bounty with a base bounty", "POST", "/bounty/p1", "gs", { bounties: [BASE], draft_data: { current_draft_choices: ["a"], previous_draft_selections: [], bronze_count: 1, silver_count: 0, gold_count: 0 } }],
    ["bounty bad body", "POST", "/bounty/p1", "gs", { bounties: {} }],
    ["bounty not an object", "POST", "/bounty/p1", "gs", "[]"],
    ["bounty malformed json", "POST", "/bounty/p1", "gs", "{nope"],
    ["bounty no body", "POST", "/bounty/p1", "gs"],
    ["bounty no-player sentinel", "POST", "/bounty/INVALID", "gs", SAVE],
    ["bounty sentinel read", "GET", "/bounty/INVALID", "gs"],
    ["bounty delete by id", "POST", "/bounty/delete/p1", "gs", { bounty_ids: ["Bounty_Bronze_X", SEASON] }],
    ["bounty delete reset", "POST", "/bounty/delete/p1", "gs"],
    ["bounty delete names none", "DELETE", "/bounty/delete/p1", "gs", { bounty_ids: [] }],
    ["bounty delete by a player", "POST", "/bounty/delete/p1", P1, { bounty_ids: ["x"] }],
    ["bounty after deletes", "GET", "/bounty/p1", "gs"],
    ["cooldown empty", "GET", "/cooldown/p1", P1],
    ["cooldown no credential", "GET", "/cooldown/p1"],
    ["cooldown batch by a player", "PUT", "/cooldown/batch/p1", P1, BATCH],
    ["cooldown batch", "PUT", "/cooldown/batch/p1", "gs", BATCH],
    ["cooldown read back", "GET", "/cooldown/p1", P1],
    ["cooldown other player", "GET", "/cooldown/p1", P2],
    ["cooldown batch bad", "PUT", "/cooldown/batch/p1", "gs", { cooldowns: [{ cooldown_id: "x" }] }],
    ["cooldown batch sentinel", "PUT", "/cooldown/batch/INVALID", "gs", BATCH],
    ["cooldown start", "PUT", `/cooldown/p1/${SPROUT}`, "gs"],
    ["cooldown start by POST with a body", "POST", `/cooldown/p1/${SPROUT}`, "gs", { x: 1 }],
    ["cooldown start, bad id", "PUT", "/cooldown/p1/bad%20id", "gs"],
    ["cooldown start, sentinel", "PUT", "/cooldown/INVALID/abc", "gs"],
    ["cooldown start by a player", "PUT", "/cooldown/p1/abc", P1],
    ["cooldown start, no credential", "PUT", "/cooldown/p1/abc"],
    ["cooldown POST on batch", "POST", "/cooldown/batch/p1", "gs"],
    ["cooldown after starts", "GET", "/cooldown/p1", "gs"],
    ["entitlements none", "GET", "/entitlementsv2", P1],
    ["entitlements no credential", "GET", "/entitlementsv2"],
    ["entitlement post", "POST", "/entitlementv2/p1", P1, {}],
    ["balance creates the wallet", "GET", "/balance", P1],
    ["balance, game server", "GET", "/balance", "gs"],
    ["balance, no credential", "GET", "/balance"],
    ["reconcile", "POST", "/reconcile", P1],
    ["creator", "GET", "/creator", P1],
    ["skus without a tag", "GET", "/product/skus/public", P1],
    ["skus with an empty tag", "GET", "/product/skus/public?requiredTags=", P1],
    ["skus with two tags", "GET", "/product/skus/public?requiredTags=a&requiredTags=b", P1],
    ["skus, no credential", "GET", "/product/skus/public?requiredTags=ladyluckstore"],
    ["skus unknown tag", "GET", "/product/skus/public?requiredTags=mailbox", P1],
    ["skus ladyluck", "GET", "/product/skus/public?requiredTags=ladyluckstore", P1],
    ["skus webstore", "GET", "/product/skus/public?requiredTags=webstore", P1],
    ["skus dyes", "GET", "/product/skus/public?requiredTags=dyes", P1],
    ["skus reward cache", "GET", "/product/skus/public?requiredTags=season_store", P1],
    ["skus exchange slot", "GET", "/product/skus/public?requiredTags=exchange_vendor_slot_2", P1],
    ["skus fountain", "GET", "/product/skus/public?requiredTags=fountain_daily_free_bundle", P1],
    ["token", "GET", "/token/markssteel/ll_sword", P1, undefined, "token"],
    ["token by item catalog id", "GET", "/token/markssteel/WP_SWORD", P1],
    ["token unknown", "GET", "/token/markssteel/nope", P1],
    ["token of another store's sku", "GET", "/token/platinum/dye_red", P1],
    ["token, no credential", "GET", "/token/markssteel/ll_sword"],
    ["notification without a token", "POST", "/notification/markssteel", P1],
    ["notification with a bad token", "POST", "/notification/markssteel?token=garbage", P1],
    ["notification", "POST", "/notification/markssteel?token={token}", P1],
    ["notification replay", "POST", "/notification/markssteel?token={token}", P1],
    ["notification by another player", "POST", "/notification/markssteel?token={token}", P2],
    ["notification, no credential", "POST", "/notification/markssteel?token={token}"],
    ["token too dear", "GET", "/token/marksgilded/ll_dragon", P1, undefined, "token"],
    ["notification too dear", "POST", "/notification/marksgilded?token={token}", P1],
    ["token of a repeatable sku", "GET", "/token/markssteel/ll_tonic", P1, undefined, "token"],
    ["notification of a repeatable sku", "POST", "/notification/markssteel?token={token}", P1],
    ["token of the fountain", "GET", "/token/platinum/fountain_bundle", P1, undefined, "token"],
    ["fountain claim", "POST", "/notification/platinum?token={token}", P1],
    ["skus fountain after the claim", "GET", "/product/skus/public?requiredTags=fountain_daily_free_bundle", P1],
    ["skus fountain, another player", "GET", "/product/skus/public?requiredTags=fountain_daily_free_bundle", P2],
    ["skus fountain, game server", "GET", "/product/skus/public?requiredTags=fountain_daily_free_bundle", "gs"],
    ["token of an exchange slot", "GET", "/token/platinum/single_exchange_slot_2", P1, undefined, "token"],
    ["exchange slot claim", "POST", "/notification/platinum?token={token}", P1],
    ["unrouted", "GET", "/nope/x", P1],
];

function Send(port, step, tokens) {
    const [name, method, rawPath, auth, body] = step;
    const Path = rawPath.replace("{token}", () => tokens.last ?? "none");
    return new Promise((resolve, reject) => {
        const Raw = body === undefined ? undefined : typeof body === "string" ? body : J(body);
        const Headers = { host: "x" };
        if (auth !== undefined) Headers["x-test-auth"] = auth;
        if (Raw !== undefined) { Headers["content-type"] = "application/json"; Headers["content-length"] = Buffer.byteLength(Raw); }
        const Req = http.request({ port, method, path: Path, headers: Headers }, (res) => {
            const Chunks = [];
            res.on("data", (c) => Chunks.push(c));
            res.on("end", () => resolve({ name, method, path: rawPath, auth: auth ?? null, body: Raw ?? null, status: res.statusCode, contentType: res.headers["content-type"] ?? null, text: Buffer.concat(Chunks).toString() }));
        });
        Req.on("error", reject);
        if (Raw !== undefined) Req.write(Raw);
        Req.end();
    });
}

(async () => {
    const Server = App.listen(0);
    await new Promise((r) => Server.once("listening", r));
    const Port = Server.address().port;
    const Tokens = {};
    const Out = [];
    for (const Step of STEPS) {
        const Result = await Send(Port, Step, Tokens);
        if (Step[2].startsWith("/token/") && Result.status === 200) {
            Tokens.last = JSON.parse(Result.text).purchaseToken;
            Result.text = J({ purchaseToken: "<token>" });
        }
        Out.push(Result);
    }
    Server.close();
    process.stdout.write(J({ version: Version, steps: Out }, null, 1) + "\n");
    process.exit(0);
})().catch((e) => { process.stderr.write(String(e.stack ?? e) + "\n"); process.exit(1); });
