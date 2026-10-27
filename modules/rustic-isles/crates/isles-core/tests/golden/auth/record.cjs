// Records what Node's whole backend answers to scenario.json (RU5 auth golden tests): the real app.ts, the real routers,
// the real middleware and the real SQLite persistence, under Node 22, with only the code mail and Discord replaced
// (as launcherDiscord.test.ts and launcherEmail.test.ts replace them). The answers are normalized (tokens, ids, times and
// random codes become placeholders) so the Rust port's can be compared with them; see tests/auth_golden.rs, which makes
// the same requests and applies the same normalization.
//
//   node --require tsx/cjs record.cjs <scenario.json> <out.json> [version]   (inside a copy of reference/1.14.7/ParadoxBackend)
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");

const [ScenarioPath, OutPath, Version = "1.14.7"] = process.argv.slice(2);
const OLD = Version === "1.12.0";
const Root = fs.mkdtempSync(path.join(os.tmpdir(), "auth-golden-"));
// A key pair made for this run: nothing is committed, and the normalized answers never hold a token
const Keys = require("node:crypto").generateKeyPairSync("rsa", {
    modulusLength: 2048,
    privateKeyEncoding: { type: "pkcs8", format: "pem" },
    publicKeyEncoding: { type: "spki", format: "pem" },
});
const B64 = (pem) => Buffer.from(pem, "utf8").toString("base64");
const RUNTIME = ["b".repeat(64), "c".repeat(64)];

Object.assign(process.env, {
    PERSISTENCE_PROVIDER: "sqlite",
    MYSTICPARADOX_SQLITE_PATH: path.join(Root, "realm.sqlite"),
    AUTH_MODE: "LAUNCHER",
    TARGET_CHANGELIST: "647472",
    API_KEY_HASH_SECRET: "test-only-api-key-hash-secret-0123456789abcdef",
    AUTH_SIGNING_PRIVKEY_B64: B64(Keys.privateKey),
    AUTH_SIGNING_PUBKEY_B64: B64(Keys.publicKey),
    GAMESERVER_API_KEYS: "gs-key-1",
    APPROVED_EXECUTABLE_SHA256: "a".repeat(64),
    LAUNCHER_UPDATE_ROOT: path.join(Root, "updates"),
    MYSTICPARADOX_OFFICIAL_PLAY: "on",
    MYSTICPARADOX_RATE_LIMIT: "off",
    LOG_LEVEL: "silent",
    NODE_ENV: "production",
    // The Rust module without its rifts crates is Node's DEDICATED_ONLY
    MYSTICPARADOX_P2P_MODULE: "off",
});

const ManifestPath = path.join(Root, "updates", "runtime", "client", "stable", "windows-x86_64", "latest.json");
fs.mkdirSync(path.dirname(ManifestPath), { recursive: true });
fs.writeFileSync(ManifestPath, JSON.stringify({
    version: "0.1.78", targetChangelist: 647472, sha256: RUNTIME[0], extraFiles: [{ name: "winmm.dll", sha256: RUNTIME[1] }],
}));

// Discord: the code "code-<id>" is the Discord user <id>
const SavedFetch = globalThis.fetch;
globalThis.fetch = (input, init) => {
    const Url = String(input);
    const Json = (body, status = 200) => Promise.resolve(new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } }));
    if (Url === "https://discord.com/api/oauth2/token") {
        const Code = new URLSearchParams(String(init?.body ?? "")).get("code") ?? "";
        if (!Code.startsWith("code-")) return Json({ error: "invalid_grant" }, 400);
        return Json({ access_token: `tok-${Code.slice(5)}`, token_type: "Bearer" });
    }
    if (Url === "https://discord.com/api/users/@me") {
        const Id = String(init?.headers?.Authorization ?? "").replace(/^Bearer tok-/, "");
        return Json({ id: Id, username: `user-${Id.slice(-4)}`, global_name: null, avatar: null });
    }
    return SavedFetch(input, init);
};

const Mails = [];
const Src = path.join(process.cwd(), "src");
let App;
let Start = async () => undefined;
const Vars = {};
if (!OLD) {
    ({ app: App } = require(path.join(Src, "app")));
    const { SetMailTransportForTest } = require(path.join(Src, "mail/mailSender"));
    SetMailTransportForTest({ async send(message) { Mails.push(message); } });
    const { GetPersistenceLifecycle } = require(path.join(Src, "persistence"));
    const { DrainAndRegisterAPIKeys } = require(path.join(Src, "controllers/apikeys"));
    Start = async () => { await GetPersistenceLifecycle().start(); await DrainAndRegisterAPIKeys(); };
} else {
    // 1.12.0 only has MongoDB: its login and matchmaking routers run over a stand-in for the persistence layer, which knows one
    // game account, "u1" (Fox), and no launcher accounts. The auth middleware and the controllers are the reference's own.
    const Resolved = require.resolve(path.join(Src, "persistence"));
    require.cache[Resolved] = {
        id: Resolved, filename: Resolved, loaded: true, children: [], paths: [],
        exports: {
            GetRepositories: () => ({
                launcherAccounts: { findByUserId: async () => undefined },
                accounts: { findByUserId: async (id) => (id === "u1" ? { userId: id, name: "Fox", notes: 0 } : undefined) },
            }),
        },
    };
    const express = require("express");
    App = express();
    App.use(express.json());
    App.use("/", require(path.join(Src, "routes/login")).loginRouter);
    App.use("/", require(path.join(Src, "routes/matchmaking")).matchmakingRouter);
    App.use((req, res) => { res.status(404); res.send(); });
    const jwt = require("jsonwebtoken");
    Vars.tok = jwt.sign({ userId: "u1" }, Keys.privateKey, {
        algorithm: "RS256", expiresIn: "24h", issuer: "paradox-backend", audience: "paradox-backend",
    });
}

// ---- the shared normalization (tests/auth_golden.rs does the same)
function NormalizeText(text) {
    return text
        .replace(/eyJ[\w-]+\.[\w-]+\.[\w-]+/g, "<jwt>")
        .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, "<uuid>")
        .replace(/(?!2085-09-09)\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z/g, "<iso>")
        .replace(/(?<![0-9a-f])[0-9a-f]{64}(?![0-9a-f])/g, "<hex64>")
        .replace(/(?<![0-9a-f])[0-9a-f]{48}(?![0-9a-f])/g, "<hex48>")
        .replace(/code_challenge=[A-Za-z0-9_-]{43}/g, "code_challenge=<challenge>");
}
function NormalizeValue(value, key) {
    if (typeof value === "string") return key === "nonce" ? "<nonce>" : NormalizeText(value);
    if (Array.isArray(value)) return value.map((item) => NormalizeValue(item));
    if (value !== null && typeof value === "object") {
        const Out = {};
        for (const [name, item] of Object.entries(value)) Out[name] = NormalizeValue(item, name);
        return Out;
    }
    return value;
}
const KEPT_HEADERS = ["content-type", "location", "cache-control", "access-control-allow-origin", "access-control-allow-methods", "access-control-allow-headers", "vary"];

function Fill(template, vars) {
    if (typeof template === "string") return template.replace(/\{\{([^}]+)\}\}/g, (_, name) => {
        if (!(name in vars)) throw new Error(`no variable ${name}`);
        return vars[name];
    });
    if (Array.isArray(template)) return template.map((item) => Fill(item, vars));
    if (template !== null && typeof template === "object") return Object.fromEntries(Object.entries(template).map(([k, v]) => [k, Fill(v, vars)]));
    return template;
}

function Send(port, step, vars, counter) {
    return new Promise((resolve, reject) => {
        const Headers = { "x-forwarded-for": `198.51.100.${counter % 250 + 1}`, ...Fill(step.headers ?? {}, vars) };
        let Payload;
        if (step.json !== undefined) { Payload = JSON.stringify(Fill(step.json, vars)); Headers["content-type"] = "application/json"; }
        else if (step.raw !== undefined) { Payload = step.raw; Headers["content-type"] = step.contentType ?? "application/json"; }
        else if (step.form !== undefined) { Payload = step.form; Headers["content-type"] = "application/x-www-form-urlencoded"; }
        const Req = http.request({ hostname: "127.0.0.1", port, path: Fill(step.path, vars), method: step.method, headers: Headers, agent: false }, (res) => {
            let Text = "";
            res.setEncoding("utf8");
            res.on("data", (chunk) => { Text += chunk; });
            res.on("end", () => resolve({ status: res.statusCode, headers: res.headers, text: Text }));
        });
        Req.on("error", reject);
        if (Payload !== undefined) Req.write(Payload);
        Req.end();
    });
}

function Capture(spec, response) {
    const Parts = spec.split("~");
    let Source;
    if (Parts[0] === "" && Parts.length === 3) Source = String(response.headers[Parts[1]] ?? "");
    else {
        let Value = JSON.parse(response.text);
        for (const key of Parts[0].split(".")) Value = Value?.[key];
        Source = String(Value);
        if (Parts.length === 1) return Source;
    }
    const Match = new RegExp(Parts[Parts.length - 1]).exec(Source);
    if (Match === null) throw new Error(`capture ${spec} found nothing in ${Source}`);
    return Match[1];
}

(async () => {
    await Start();
    const Listening = App.listen(0, "127.0.0.1");
    await new Promise((resolve) => Listening.once("listening", resolve));
    const Port = Listening.address().port;
    const Scenario = JSON.parse(fs.readFileSync(ScenarioPath, "utf8"));
    const Out = [];
    let Counter = 0;
    for (const step of Scenario.steps) {
        if (step.versions !== undefined && !step.versions.includes(Version)) continue;
        if (step.env !== undefined) { Object.assign(process.env, step.env); Out.push({ name: step.name, env: step.env }); continue; }
        if (step.mailcode !== undefined) {
            const Mail = [...Mails].reverse().find((m) => m.to === step.mailcode.email);
            const Code = /(\d{6})$/.exec(Mail?.subject ?? "")?.[1];
            if (Code === undefined) throw new Error(`no mail to ${step.mailcode.email}`);
            Vars[step.mailcode.var] = Code;
            Out.push({ name: step.name, mailcode: step.mailcode });
            continue;
        }
        Counter += 1;
        const Response = await Send(Port, step, Vars, Counter);
        for (const [name, spec] of Object.entries(step.capture ?? {})) {
            if (Response.status < 400 || spec.startsWith("~")) Vars[name] = Capture(spec, Response);
        }
        const Type = Response.headers["content-type"] ?? "";
        let Body;
        if (/json/.test(Type)) { try { Body = JSON.stringify(NormalizeValue(JSON.parse(Response.text))); } catch { Body = NormalizeText(Response.text); } }
        else Body = NormalizeText(Response.text);
        const Headers = {};
        for (const name of KEPT_HEADERS) if (Response.headers[name] !== undefined) Headers[name] = NormalizeText(String(Response.headers[name]));
        Out.push({ name: step.name, status: Response.status, headers: Headers, body: Body });
    }
    Listening.close();
    fs.writeFileSync(OutPath, JSON.stringify(Out, null, 1) + "\n");
    process.exit(0);
})().catch((e) => { console.error(e); process.exit(1); });
