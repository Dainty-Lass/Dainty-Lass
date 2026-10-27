// Runs Mystic Paradox's own admission, relay and ICE code and writes what it answers, for the Rust port's tests
// (node-admission.json here, rifts-relay's tests/vectors/node-relay.json). Its inputs, beside this script: rustic-isles'
// reference/1.14.7/ParadoxBackend src/p2p/{security/admissionTickets,security/iceSessionRules,security/transportPeerIds,
// services/relayCredentials,config/p2pIcePolicy}.ts under src/p2p/, unchanged but for their import specifiers (".ts"
// added, the type-only imports marked `type`, `../realm/realmHubRules` replaced by a stub exporting
// PRESENCE_LIVE_STATES), a stub src/logger.ts, and a key set from scripts/p2p/generate-admission-key.mjs. Then:
//   docker run --rm -v "$PWD:/work" -w /work node:22 node --experimental-strip-types make-vectors.mts <keys> <out>
// Only the key set's public key goes into the vectors.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const [keysDir, outDir] = process.argv.slice(2);
const env = Object.fromEntries(fs.readFileSync(path.join(keysDir, "metagame-admission.env"), "utf8")
    .split("\n").filter(Boolean).map((line) => [line.slice(0, line.indexOf("=")), line.slice(line.indexOf("=") + 1)]));
Object.assign(process.env, env);

const { IssueAdmissionTicket, VerifyAdmissionTicket, HashAdmissionJti } = await import("./src/p2p/security/admissionTickets.ts");
const relay = await import("./src/p2p/services/relayCredentials.ts");
const policy = await import("./src/p2p/config/p2pIcePolicy.ts");
const ice = await import("./src/p2p/security/iceSessionRules.ts");
const peers = await import("./src/p2p/security/transportPeerIds.ts");

const NOW = 1_800_000_000;
const privateKey = crypto.createPrivateKey(Buffer.from(env.P2P_ADMISSION_SIGNING_PRIVKEY_B64, "base64").toString("utf8"));

function verdict(token: string, now: number) {
    try {
        const claims = VerifyAdmissionTicket(token, now);
        return { token, now, ok: claims, error: null };
    } catch (error) {
        return { token, now, ok: null, error: (error as Error).message };
    }
}

function signed(payload: Buffer): string {
    const signature = crypto.sign("sha256", payload, { key: privateKey, dsaEncoding: "ieee-p1363" });
    return `mpt1.${payload.toString("base64url")}.${signature.toString("base64url")}`;
}

const base = {
    accountId: "account-1", joinerSteamId: "76561198000000001", hostSteamId: "76561198000000002",
    sessionId: "01234567-89ab-4def-8123-456789abcdef", generation: 7, partyId: "party_abc==", partyRevision: 12,
    huntDefinitionId: "hunt/island-1", build: 392819, bridgeProtocol: 1, virtualPort: 1, nowSeconds: NOW, lifetimeSeconds: 75
};
const variants = [
    base,
    { ...base, virtualPort: 999, lifetimeSeconds: 180 },
    { ...base, generation: 0, build: 0, bridgeProtocol: 0, partyRevision: 0, lifetimeSeconds: 15 },
    { ...base, accountId: "a".repeat(96), huntDefinitionId: "h:/.-_".repeat(21) + "xx", partyId: "p+/=:._-".repeat(24),
        generation: 0xffffffff, build: 0xffffffff, bridgeProtocol: 0xffff, partyRevision: Number.MAX_SAFE_INTEGER,
        joinerSteamId: "18446744073709551615", hostSteamId: "1", sessionId: "FFFFFFFF-FFFF-5FFF-BFFF-FFFFFFFFFFFF" },
    { ...base, sessionId: "00000000-0000-1000-8000-000000000000", accountId: "4f6a2d1e-0000-4000-8000-000000000001" }
];
const issued = variants.map((input) => {
    const ticket = IssueAdmissionTicket(input);
    return { input, token: ticket.token, claims: ticket.claims, jtiDigest: ticket.jtiDigest };
});

const vectors: any[] = [];
for (const ticket of issued) {
    for (const now of [NOW - 31, NOW - 30, NOW, ticket.claims.expiresAt + 30, ticket.claims.expiresAt + 31]) {
        vectors.push(verdict(ticket.token, now));
    }
}
// Every byte of the first payload bent two ways, re-signed, so the decoder's checks are reached.
const payload = Buffer.from(issued[0].token.split(".")[1], "base64url");
for (let at = 0; at < payload.length; at++) {
    for (const mask of [0x01, 0x80]) {
        const bent = Buffer.from(payload);
        bent[at] ^= mask;
        vectors.push(verdict(signed(bent), NOW));
    }
}
// Targeted payloads: inexact times, a trailing byte, a short payload, an empty identifier.
const at = (buffer: Buffer, offset: number, value: bigint) => { const copy = Buffer.from(buffer); copy.writeBigUInt64BE(value, offset); return copy; };
for (const bent of [
    at(payload, 34, (1n << 60n) + 1n), at(payload, 34, 1n << 60n), at(payload, 50, (1n << 64n) - 1n),
    at(payload, 26, 1n << 53n), at(payload, 58, 0n), Buffer.concat([payload, Buffer.from("x")]), payload.subarray(0, 105)
]) {
    vectors.push(verdict(signed(bent), NOW));
}
// Token-level forms, not re-signed.
const [prefix, body, signature] = issued[0].token.split(".");
const sig = Buffer.from(signature, "base64url");
const flipped = Buffer.from(sig); flipped[10] ^= 1;
// High S: s' = n - s.
const n = 0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551n;
const s = BigInt("0x" + sig.subarray(32).toString("hex"));
const highS = Buffer.concat([sig.subarray(0, 32), Buffer.from((n - s).toString(16).padStart(64, "0"), "hex")]);
for (const token of [
    `${prefix}.${body}.${signature}=`, `${prefix}.${body}=.${signature}`, `${prefix}.${body.replaceAll("-", "+").replaceAll("_", "/")}.${signature}`,
    `${prefix}.${body}.${signature.slice(0, 85)}`, `mpt2.${body}.${signature}`, `${prefix}.${body}`, `${prefix}..${signature}`,
    `${prefix}.${body}.${signature}.x`, `${prefix}.${body}.${flipped.toString("base64url")}`, `${prefix}.${body}.${highS.toString("base64url")}`,
    `${prefix}.${body}.${Buffer.concat([Buffer.alloc(32), sig.subarray(32)]).toString("base64url")}`, "a".repeat(4097), ""
]) {
    vectors.push(verdict(token, NOW));
}

const jtis = ["00000000000000000000000000000000", "0123456789ABCDEF0123456789abcdef", "ffffffffffffffffffffffffffffffff"];
const admission = {
    keyId: env.P2P_ADMISSION_KEY_ID,
    verifyKeyB64: env.P2P_ADMISSION_VERIFY_PUBKEY_B64,
    publicXyHex: fs.readFileSync(path.join(keysDir, "p2p-admission-public.xy")).toString("hex"),
    issued,
    vectors,
    jtiDigests: jtis.map((jti) => ({ jti, digest: HashAdmissionJti(jti) })),
    icePeerIds: ["account-1", "4f6a2d1e-0000-4000-8000-000000000001", "", "é"].map((account) => ({ account, peerId: peers.DeriveIcePeerId(account) }))
};

const accounts = ["4f6a2d1e-0000-4000-8000-000000000001", "host-0000-account", "join-aaaa-account", "é ü"];
const turnRest = { urls: ["turn:turn.example.org:3478"], secret: "x".repeat(16) };
const signals: any[] = [
    { generation: 3, toAccountId: "host-0000-account", kind: "CANDIDATE", data: "a=candidate:1 1 UDP 1 203.0.113.5 50000 typ srflx" },
    { generation: 3.0, toAccountId: "host-0000-account", kind: "GATHERING_DONE" },
    { generation: 3, toAccountId: "host-0000-account", kind: "GATHERING_DONE", data: null },
    { generation: -1, toAccountId: "host-0000-account", kind: "CANDIDATE", data: "x" },
    { generation: "3", toAccountId: "host-0000-account", kind: "CANDIDATE", data: "x" },
    { generation: 3, toAccountId: "short", kind: "CANDIDATE", data: "x" },
    { generation: 3, toAccountId: "host-0000-account", kind: "DESCRIPTION", data: " \r\n " },
    { generation: 3, toAccountId: "host-0000-account", kind: "DESCRIPTION", data: "x".repeat(4096) },
    { generation: 3, toAccountId: "host-0000-account", kind: "DESCRIPTION", data: "x".repeat(4097) },
    { generation: 3, toAccountId: "host-0000-account", kind: "CANDIDATE", data: "tab\there" },
    { generation: 9007199254740992, toAccountId: "host-0000-account", kind: "GATHERING_DONE" },
    { generation: 3, toAccountId: "host-0000-account", kind: "CANDIDATE", data: 5 }
];
const filters = [
    ["DESCRIPTION", ["c=IN IP4 192.168.1.20", "a=ice-ufrag:abcd", "a=ice-pwd:0123456789abcdef0123456789", "a=ice-options:ice2,trickle",
        "a=candidate:1 1 UDP 2122317823 192.168.1.20 50000 typ host", "a=candidate:3 1 UDP 41885439 104.30.1.1 3478 typ relay raddr 203.0.113.5 rport 50000",
        "a=end-of-candidates"].join("\r\n")],
    ["DESCRIPTION", "  a=ice-ufrag:x \n\ncandidate:9 1 UDP 1 1.2.3.4 9 typ relay\na=end-of-candidates \r\r\na=mid:0"],
    ["DESCRIPTION", "a=candidate:3 1 UDP 1 104.30.1.1 3478 typ relay"],
    ["CANDIDATE", "a=candidate:3 1 UDP 41885439 104.30.1.1 3478 typ relay"],
    ["CANDIDATE", "candidate:3 1 UDP 1 1.2.3.4 9 typ relayed"],
    ["CANDIDATE", "a=candidate:1 typ relay\na=candidate:2 typ relay"],
    ["CANDIDATE", " a=candidate:2 x typ relay "],
    ["GATHERING_DONE", ""]
];
const providerAnswers = [
    { iceServers: [{ urls: ["stun:stun.cloudflare.com:3478"] }, { urls: ["turn:turn.cloudflare.com:3478?transport=udp", "turn:turn.cloudflare.com:53?transport=udp",
        "turns:turn.cloudflare.com:5349?transport=tcp", "turn:turn.cloudflare.com:53"], username: "u".repeat(96), credential: "c".repeat(96) }] },
    { iceServers: { urls: "turn:a.example:3478", username: "user", credential: "pass" } },
    { iceServers: [{ urls: ["turn:a.example"], username: "a b", credential: "x" }, { urls: ["turn:b.example"], username: "ok", credential: "ok" }] },
    { iceServers: [{ urls: ["stun:x.example", "stun:x.example", 5, "stuns:y.example:5349"] }] },
    { iceServers: null }, {}, null, { iceServers: 7 }
];
const envs: Record<string, string>[] = [
    {},
    { P2P_TURN_MAX_TTL_SECONDS: "999999", P2P_RELAY_MONTHLY_QUOTA_GB: "-1", P2P_TRANSPORT_KEY_SECRET: "short", P2P_ICE_STUN_URLS: "http://evil, stun:stun.example.org:3478" },
    { P2P_TURN_PROVIDER: " TURN-REST ", P2P_TURN_REST_URLS: "turn:turn.example.org:3478?transport=udp, turns:x:5349?transport=tcp,turn:bad:123456", P2P_TURN_REST_SECRET: " " + "x".repeat(16) + " " },
    { P2P_TURN_PROVIDER: "turn-rest", P2P_TURN_REST_URLS: "turn:turn.example.org", P2P_TURN_REST_SECRET: "x".repeat(15) },
    { P2P_CLOUDFLARE_TURN_KEY_ID: "0123456789abcdef", P2P_CLOUDFLARE_TURN_API_TOKEN: "token-value-123456", P2P_CLOUDFLARE_ANALYTICS_ACCOUNT_ID: "abcdef0123456789",
        P2P_CLOUDFLARE_ANALYTICS_API_TOKEN: "analytics-token-1", P2P_RELAY_MONTHLY_QUOTA_GB: "0", P2P_ICE_CONFIGS_PER_HOUR: "1e2", P2P_TURN_MAX_TTL_SECONDS: "0x12c" },
    { P2P_TURN_PROVIDER: "none", P2P_TRANSPORT_KEY_SECRET: "k".repeat(32), P2P_REALM_HUB_TURN_TTL_SECONDS: " 300 ", P2P_ICE_CONFIGS_PER_HOUR: "12.0" },
    { P2P_CLOUDFLARE_TURN_KEY_ID: "short", P2P_CLOUDFLARE_TURN_API_TOKEN: "token value with spaces" }
];
const usageQueries: any[] = [];
const analytics = { accountId: "abcdef0123456789abcdef0123456789", apiToken: "analytics-token-1" };
for (const account of accounts.slice(0, 2)) {
    await relay.ReadMonthlyRelayEgressBytes(analytics, account, new Date(NOW * 1000), async (url: string, init: any) => {
        usageQueries.push({ account, url, body: init.body });
        return { status: 500, json: async () => undefined };
    });
}
const relayVectors = {
    tags: accounts.map((account) => ({ account, tag: relay.RelayAccountTag(account) })),
    turnRest: accounts.map((account) => ({ account, nowMs: NOW * 1000 + 999, ttl: 600, secret: turnRest.secret,
        ...relay.MintTurnRestCredentials(turnRest, account, 600, NOW * 1000 + 999) })),
    pairKeys: accounts.map((joiner) => ({ secret: "s".repeat(40), sessionId: "01234567-89ab-4def-8123-456789abcdef", generation: 3,
        host: "host-0000-account", joiner, key: ice.DeriveIcePairKey("s".repeat(40), "01234567-89ab-4def-8123-456789abcdef", 3, "host-0000-account", joiner) })),
    signals: signals.map((body) => ({ body, parsed: ice.ParseIceSignal(body) ?? null })),
    filters: filters.map(([kind, data]) => ({ kind, data, kept: ice.FilterRelayOnly(kind as any, data) ?? null })),
    provider: providerAnswers.map((body) => ({ body, normalized: relay.NormalizeProviderIceServers(body) })),
    configs: envs.map((environment) => {
        const config = policy.ReadP2PIceConfig(environment as any);
        return { env: environment, summary: policy.P2PIceSummary(config), stunUrls: config.stunUrls, turnRestUrls: config.turnRest?.urls ?? null,
            cloudflareKeyId: config.cloudflare?.keyId ?? null, maxTtl: config.maxCredentialTtlSeconds, hubTtl: config.realmHubCredentialTtlSeconds,
            configsPerHour: config.configsPerHour, quotaBytes: config.monthlyRelayQuotaBytes, analytics: config.analytics?.accountId ?? null };
    }),
    usageQueries
};

fs.mkdirSync(outDir, { recursive: true });
fs.writeFileSync(path.join(outDir, "admission.json"), JSON.stringify(admission, null, 1) + "\n");
fs.writeFileSync(path.join(outDir, "relay.json"), JSON.stringify(relayVectors, null, 1) + "\n");
console.log(`wrote ${vectors.length} ticket verdicts, ${issued.length} issued tickets and the relay vectors to ${outDir}`);
