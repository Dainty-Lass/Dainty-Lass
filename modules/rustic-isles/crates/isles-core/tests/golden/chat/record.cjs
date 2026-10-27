// Records what Node's friends and guild routers answer (RU5 chat golden tests). Run inside a copy of
// reference/1.14.7/ParadoxBackend after `npm ci`, as `node --require tsx/cjs record.cjs > chat.json`; see run.sh.
// The auth middleware, the logger and the friendship store are replaced as friendsSlayerLink.test.ts replaces them;
// the routers are Node's own. The clock is fixed so the timestamps compare.
const http = require("node:http");
const path = require("node:path");

const Quiet = () => undefined;
function Fake(modulePath, exports) {
    const Resolved = require.resolve(modulePath);
    require.cache[Resolved] = { id: Resolved, filename: Resolved, loaded: true, exports, children: [], paths: [] };
}
const Src = path.join(process.cwd(), "src");
Fake(path.join(Src, "logger"), { logger: { level: "silent", trace: Quiet, debug: Quiet, info: Quiet, warn: Quiet, error: Quiet, fatal: Quiet, child: () => ({}) }, ApplyDiagnosticsLogProfile: Quiet });
Fake(path.join(Src, "middleware/HasParadoxBackendAuth"), {
    HasParadoxBackendAuth: (req, res, next) => {
        const User = req.headers["x-test-user"];
        if (User === undefined) { res.status(401); res.send(); return; }
        req.AuthData = User === "" ? {} : { userId: User };
        next();
    },
});

// An insertion-ordered map, as the Mongo collection lists (natural order) and the SQLite doc table does.
const Edges = new Map();
const Key = (o, p) => `${o}:${p}`;
Fake(path.join(Src, "persistence"), {
    GetRepositories: () => ({
        friendships: {
            async listForOwner(owner) { return [...Edges.values()].filter((e) => e.ownerId === owner); },
            async find(owner, other) { return Edges.get(Key(owner, other)); },
            async upsert(edge) {
                const Fields = { ownerId: edge.ownerId, otherId: edge.otherId, status: edge.status, direction: edge.direction ?? undefined, created: edge.created, favorite: edge.favorite ?? false };
                Edges.set(Key(edge.ownerId, edge.otherId), Fields);
            },
            async remove(owner, other) { Edges.delete(Key(owner, other)); },
        },
    }),
});

// Date: new Date() is the next second of a fixed run; everything else is the real Date.
const RealDate = Date;
let Tick = 0;
global.Date = class extends RealDate {
    constructor(...args) {
        if (args.length === 0) super(RealDate.UTC(2026, 9, 20, 12, 0, Tick++));
        else super(...args);
    }
};

const express = require("express");
const { friendsRouter } = require(path.join(Src, "routes/friends"));
const { guildRouter } = require(path.join(Src, "routes/guild"));

const App = express();
App.use(express.json());
App.use(guildRouter);
App.use(friendsRouter);

function Send(port, step) {
    return new Promise((resolve, reject) => {
        const Headers = {};
        if (step.user !== undefined) Headers["x-test-user"] = step.user;
        let Payload;
        if (step.raw !== undefined) { Payload = step.raw; Headers["content-type"] = step.contentType ?? "application/json"; }
        else if (step.body !== undefined) { Payload = JSON.stringify(step.body); Headers["content-type"] = "application/json"; }
        const Req = http.request({ hostname: "127.0.0.1", port, path: step.path, method: step.method, headers: Headers }, (res) => {
            let Text = "";
            res.setEncoding("utf8");
            res.on("data", (Chunk) => { Text += Chunk; });
            res.on("end", () => resolve({ status: res.statusCode, contentType: res.headers["content-type"] ?? null, body: Text }));
        });
        Req.on("error", reject);
        if (Payload !== undefined) Req.write(Payload);
        Req.end();
    });
}

const Steps = [
    { name: "list, nothing yet", method: "GET", path: "/friends/api/public/friends/alice", user: "alice" },
    { name: "list without a bearer", method: "GET", path: "/friends/api/public/friends/alice" },
    { name: "list with an empty identity", method: "GET", path: "/friends/api/public/friends/x", user: "" },
    { name: "invite", method: "POST", path: "/friends/api/public/friends/alice/bob", user: "alice" },
    { name: "invite again", method: "POST", path: "/friends/api/public/friends/alice/bob", user: "alice" },
    { name: "invite oneself", method: "POST", path: "/friends/api/public/friends/alice/alice", user: "alice" },
    { name: "invite without a bearer", method: "POST", path: "/friends/api/public/friends/alice/bob" },
    { name: "list hides pending", method: "GET", path: "/friends/api/public/friends/alice", user: "alice" },
    { name: "list with pending", method: "GET", path: "/friends/api/public/friends/alice?includePending=true", user: "alice" },
    { name: "list with pending, bracket spelling", method: "GET", path: "/friends/api/public/friends/alice?includePending[]=true", user: "alice" },
    { name: "list with pending twice", method: "GET", path: "/friends/api/public/friends/alice?includePending=true&includePending=true", user: "alice" },
    { name: "list with pending, not true", method: "GET", path: "/friends/api/public/friends/alice?includePending=TRUE", user: "alice" },
    { name: "the invited sees it inbound", method: "GET", path: "/friends/api/public/friends/bob?includePending=true", user: "bob" },
    { name: "a second invite", method: "POST", path: "/friends/api/public/friends/alice/carol", user: "alice" },
    { name: "accept", method: "POST", path: "/friends/api/public/friends/bob/alice", user: "bob" },
    { name: "accept again", method: "POST", path: "/friends/api/public/friends/bob/alice", user: "bob" },
    { name: "alice's list", method: "GET", path: "/friends/api/public/friends/alice?includePending=true", user: "alice" },
    { name: "bob's list", method: "GET", path: "/friends/api/public/friends/bob", user: "bob" },
    { name: "accepted friend invited again", method: "POST", path: "/friends/api/public/friends/alice/bob", user: "alice" },
    { name: "remove a friend", method: "DELETE", path: "/friends/api/public/friends/alice/bob", user: "alice" },
    { name: "remove nobody", method: "DELETE", path: "/friends/api/public/friends/alice/nobody", user: "alice" },
    { name: "remove without a bearer", method: "DELETE", path: "/friends/api/public/friends/alice/carol" },
    { name: "alice after", method: "GET", path: "/friends/api/public/friends/alice?includePending=true", user: "alice" },
    { name: "carol rejects", method: "DELETE", path: "/friends/api/public/friends/carol/alice", user: "carol" },
    { name: "alice at the end", method: "GET", path: "/friends/api/public/friends/alice?includePending=true", user: "alice" },
    { name: "blocklist", method: "GET", path: "/friends/api/public/blocklist/alice" },
    { name: "recent players", method: "GET", path: "/friends/api/public/list/fortnite/alice/recentPlayers" },
    { name: "settings", method: "GET", path: "/friends/api/v1/alice/settings" },
    { name: "guild invites", method: "GET", path: "/guild/invite/player", user: "alice" },
    { name: "guild invites without a bearer", method: "GET", path: "/guild/invite/player" },
    { name: "guild", method: "GET", path: "/guild", user: "alice" },
    { name: "guild without a bearer", method: "GET", path: "/guild" },
    { name: "availability", method: "POST", path: "/slayerlink/availability", user: "alice", body: { account_ids: ["XQBF5VHGFJHR5AUILEPYI55MUQ", "", 7, "PLNTNCL7PNFUHBRPEBVSTAZ47A", null, "é", "😀"] } },
    { name: "availability, emoji is two units", method: "POST", path: "/slayerlink/availability", user: "alice", body: { account_ids: ["😀".repeat(64), "😀".repeat(65), "x".repeat(128), "x".repeat(129)] } },
    { name: "availability, not a list", method: "POST", path: "/slayerlink/availability", user: "alice", body: { account_ids: "not a list" } },
    { name: "availability, no body", method: "POST", path: "/slayerlink/availability", user: "alice" },
    { name: "availability, an array body", method: "POST", path: "/slayerlink/availability", user: "alice", body: ["a"] },
    { name: "availability, many", method: "POST", path: "/slayerlink/availability", user: "alice", body: { account_ids: Array.from({ length: 300 }, (_, i) => `ACCOUNT${i}`) } },
    { name: "availability, escapes", method: "POST", path: "/slayerlink/availability", user: "alice", raw: "{\"account_ids\":[\"a\\\"b\\\\c\\n\\u0001\\u007f/\\u2028\"]}" },
    { name: "availability without a bearer", method: "POST", path: "/slayerlink/availability", body: { account_ids: [] } },
    { name: "availability, text content type", method: "POST", path: "/slayerlink/availability", user: "alice", raw: "{\"account_ids\":[\"a\"]}", contentType: "text/plain" },
    { name: "availability, empty json body", method: "POST", path: "/slayerlink/availability", user: "alice", raw: "" },
    { name: "status", method: "GET", path: "/slayerlink/status_good", user: "alice" },
    { name: "status without a bearer", method: "GET", path: "/slayerlink/status_good" },
];

(async () => {
    const Listening = App.listen(0, "127.0.0.1");
    await new Promise((resolve) => Listening.once("listening", resolve));
    const Port = Listening.address().port;
    const Out = [];
    for (const step of Steps) {
        const Got = await Send(Port, step);
        Out.push({ name: step.name, method: step.method, path: step.path, user: step.user ?? null, body: step.raw !== undefined ? { raw: step.raw, contentType: step.contentType ?? "application/json" } : (step.body ?? null), expect: Got });
    }
    Listening.close();
    process.stdout.write(JSON.stringify(Out, null, 1) + "\n");
})().catch((e) => { console.error(e); process.exit(1); });
