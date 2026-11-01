// RU5 reforged-api golden capture: mounts the reference routers of routes/reforgedApi.ts, loginQueue.ts, metrics.ts,
// releaseGates.ts, launcherUpdates.ts, launcherLogs.ts and adminLauncherLogs.ts, unchanged, on a real Express app
// (the body parsers and the trust-proxy setting app.ts installs) under Node 22, and records what each case gets:
// status, the headers a client sees, the body text. The environment and the files on disk are the case's own; every
// case imports the routers afresh because they read part of their settings when loaded. The output,
// reforged_api.json, is what the Rust handlers are compared with (tests/reforged_api_golden.rs).
//
//   docker run --rm -v "$PWD":/repo -w /work node:22 sh -c '
//     cp -r /repo/reference/1.14.7/ParadoxBackend/{package.json,package-lock.json,tsconfig.json,src} . &&
//     npm ci --ignore-scripts --no-audit --no-fund && npm i --no-save --ignore-scripts tsx &&
//     npx tsx /repo/crates/isles-core/tests/golden/reforged_api_capture.mts /work /repo/crates/isles-core/tests/golden'
//
// Not captured, and why: the routes behind a database (Mongo is not started) and the signed publishes (the release
// private keys are not ours); both are checked by tests/reforged_api.rs and tests/reforged_updates.rs against the
// reference code read by hand. The gates that run before the database are captured.

import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

const [workRoot, outDir] = process.argv.slice(2);
const NOW = Date.parse("2026-10-26T12:00:00.000Z");
Date.now = () => NOW;

type Case = {
    name: string;
    env?: Record<string, string>;
    files?: Record<string, string>;
    request: { method: string; path: string; headers?: Record<string, string>; json?: string };
    /** The body is not compared (it holds a clock or a counter). */
    skipBody?: boolean;
};

const ADMIN_KEY = "0123456789abcdef0123456789abcdef0123";
const PUBLISHER = { UPDATE_PUBLISHER_ALLOWED_IPS: "127.0.0.1", UPDATE_PUBLISHER_API_KEY: "publisher-key" };
const BASE = { UPDATE_PUBLIC_BASE_URL: "https://updates.example.test/" };
const FEED = "runtime/client/stable/windows-x86_64";
const manifest = (extra: object = {}) => JSON.stringify({
    schema: 1, component: "ParadoxRuntime", target: "client", version: "1.2.3", channel: "stable", targetChangelist: 647472,
    platform: "windows-x86_64", size: 3, sha256: "00", signature: "sig", file: "ParadoxRuntime-1.2.3-client-647472.dll",
    publishedAt: "2026-10-01T00:00:00.000Z", url: "https://updates.example.test/x", extraFiles: [{ name: "winmm.dll", size: 5, sha256: "11", signature: "s2", url: "u" }],
    ...extra
});
const runtimeFiles = (extra: object = {}): Record<string, string> => ({
    [`${FEED}/latest.json`]: manifest(extra),
    [`${FEED}/1.2.3/ParadoxRuntime-1.2.3-client-647472.dll`]: "dll",
    [`${FEED}/1.2.3/winmm.dll`]: "winmm",
});
const launcherManifest = (extra: object = {}) => JSON.stringify({
    version: "0.2.0", notes: "n", pub_date: "2026-10-20T00:00:00Z",
    url: "https://updates.example.test/launcher/v1/updates/windows/x86_64/files/0.2.0/App_0.2.0_x64-setup.exe",
    signature: "sig", file: "App_0.2.0_x64-setup.exe", sha256: "00", size: 3, ...extra
});
const launcherFiles = (extra: object = {}): Record<string, string> => ({
    "launcher/windows/x86_64/latest.json": launcherManifest(extra),
    "launcher/windows/x86_64/0.2.0/App_0.2.0_x64-setup.exe": "exe",
});
const get = (p: string, headers?: Record<string, string>) => ({ method: "GET", path: p, headers });
const post = (p: string, json?: string, headers?: Record<string, string>) => ({ method: "POST", path: p, json, headers });
const host = { "x-reforged-admin-key": ADMIN_KEY };
const SESSION = "abcdef12-3456-7890";
const SYNC = { LAUNCHER_LOG_SYNC_TOKEN: "sync-token" };
const sync = { "x-sync-token": "sync-token" };
const logFiles = {
    [`logs/u1/${SESSION}/launcher.log`]: "log text",
    [`logs/u1/${SESSION}/metadata.json`]: "{}",
    [`logs/u1/${SESSION}/.uploaded-at.json`]: '{"lastUploadedAt":"2026-10-02T00:00:00.000Z"}\n',
    [`logs/u2/bbbbbbbb-2/launcher.log`]: "other",
    [`logs/u2/bbbbbbbb-2/.uploaded-at.json`]: '{"lastUploadedAt":"2026-10-01T00:00:00.000Z"}\n',
};

const CASES: Case[] = [
    { name: "maintenance status", request: get("/maintenance/status") },

    // metrics: the body holds the clock
    { name: "metrics, loopback without a token", env: { NODE_ENV: "development" }, request: get("/internal/v1/metrics"), skipBody: true },
    { name: "metrics, production without a token", env: { NODE_ENV: "production" }, request: get("/internal/v1/metrics") },
    { name: "metrics, token required and missing", env: { RUSTIC_ISLES_METRICS_TOKEN: "route-test-token" }, request: get("/internal/v1/metrics") },
    { name: "metrics, wrong bearer", env: { RUSTIC_ISLES_METRICS_TOKEN: "route-test-token" }, request: get("/internal/v1/metrics", { authorization: "Bearer nope" }) },
    { name: "metrics, bearer", env: { RUSTIC_ISLES_METRICS_TOKEN: "route-test-token" }, request: get("/internal/v1/metrics", { authorization: "Bearer route-test-token" }), skipBody: true },
    { name: "metrics, token header", env: { RUSTIC_ISLES_METRICS_TOKEN: "route-test-token" }, request: get("/internal/v1/metrics", { "x-rustic-isles-metrics-token": "route-test-token" }), skipBody: true },

    // release gates
    { name: "release gates, defaults", env: { RUSTIC_ISLES_ADMIN_API: "on" }, request: get("/health/release-gates") },
    { name: "release gates, legacy key and a bad guard mode", env: { RUSTIC_ISLES_ADMIN_API: "on", MYSTPAX_OLD: "1", MYSTPAX_AGAIN: "2", RUSTIC_ISLES_GUARD_MODE: "sometimes" }, request: get("/health/release-gates") },
    { name: "release gates, enforce and required", env: { RUSTIC_ISLES_ADMIN_API: "on", RUSTIC_ISLES_GUARD_MODE: " Enforce ", RUSTIC_ISLES_REGISTRATION_APPROVAL: "required", RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE: "true" }, request: get("/health/release-gates") },
    { name: "release gates, proxy without a bind host", env: { RUSTIC_ISLES_ADMIN_API: "on", RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY: "on" }, request: get("/health/release-gates") },
    { name: "release gates, proxy on loopback", env: { RUSTIC_ISLES_ADMIN_API: "on", RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY: "on", BIND_HOST: "127.0.0.1" }, request: get("/health/release-gates") },
    { name: "release gates, proxy value unreadable", env: { RUSTIC_ISLES_ADMIN_API: "on", RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY: "perhaps" }, request: get("/health/release-gates") },
    { name: "release gates, http only", env: { RUSTIC_ISLES_ADMIN_API: "on", REFORGED_HTTP_ONLY: "yes", BIND_HOST: "0.0.0.0" }, request: get("/health/release-gates") },
    { name: "release gates, forwarded", env: { RUSTIC_ISLES_ADMIN_API: "on" }, request: get("/health/release-gates", { "x-forwarded-for": "203.0.113.7" }) },
    { name: "release gates, real ip", env: { RUSTIC_ISLES_ADMIN_API: "on" }, request: get("/health/release-gates", { "x-real-ip": "203.0.113.7" }) },
    { name: "release gates, forwarded header", env: { RUSTIC_ISLES_ADMIN_API: "on" }, request: get("/health/release-gates", { forwarded: "for=203.0.113.7" }) },

    // the Reforged API: the answers that come before the database
    { name: "registration status, default", request: get("/reforged/api/RegistrationStatus") },
    { name: "registration status, open", env: { REGISTRATION_MODE: " open " }, request: get("/reforged/api/RegistrationStatus") },
    { name: "registration status, none", env: { REGISTRATION_MODE: "NONE" }, request: get("/reforged/api/RegistrationStatus") },
    { name: "registration status, unknown", env: { REGISTRATION_MODE: "yes please" }, request: get("/reforged/api/RegistrationStatus") },
    { name: "register, closed", env: { REGISTRATION_MODE: "NONE" }, request: post("/reforged/api/Register", '{"Username":"Fox_9"}') },
    { name: "register, no body", env: { REGISTRATION_MODE: "OPEN" }, request: post("/reforged/api/Register") },
    { name: "register, name not text", env: { REGISTRATION_MODE: "OPEN" }, request: post("/reforged/api/Register", '{"Username":5}') },
    { name: "register, code not text", env: { REGISTRATION_MODE: "OPEN" }, request: post("/reforged/api/Register", '{"Username":"Fox_9","InviteCode":7}') },
    { name: "register, code null", env: { REGISTRATION_MODE: "OPEN" }, request: post("/reforged/api/Register", '{"Username":"Fox_9","InviteCode":null}') },
    { name: "register, name too short", env: { REGISTRATION_MODE: "OPEN" }, request: post("/reforged/api/Register", '{"Username":" ab "}') },
    { name: "register, name with a space", env: { REGISTRATION_MODE: "OPEN" }, request: post("/reforged/api/Register", '{"Username":"Fox 9"}') },
    { name: "register, name too long", env: { REGISTRATION_MODE: "OPEN" }, request: post("/reforged/api/Register", '{"Username":"abcdefghijklmnopq"}') },
    { name: "register, invite needed", request: post("/reforged/api/Register", '{"Username":"Fox_9"}') },
    { name: "register, invite malformed", request: post("/reforged/api/Register", '{"Username":"Fox_9","InviteCode":"a b"}') },
    { name: "register, invite too short", request: post("/reforged/api/Register", '{"Username":"Fox_9","InviteCode":"abc"}') },
    { name: "register, broken json", env: { REGISTRATION_MODE: "OPEN", NODE_ENV: "production" }, request: post("/reforged/api/Register", '{"Username":') },
    { name: "get user info, no key", request: get("/reforged/api/GetUserInfo") },
    { name: "get user info, odd key", request: get("/reforged/api/GetUserInfo", { "x-reforged-user-api-key": "no good" }) },
    { name: "create invite, not the host", request: post("/reforged/api/CreateInvite", "{}") },
    { name: "create invite, wrong key", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: post("/reforged/api/CreateInvite", "{}", { "x-reforged-admin-key": "wrong" }) },
    { name: "create invite, gateway stamp", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: post("/reforged/api/CreateInvite", "{}", { ...host, "x-reforged-gateway": "1" }) },
    { name: "create invite, forwarded", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: post("/reforged/api/CreateInvite", "{}", { ...host, "x-forwarded-for": "203.0.113.9" }) },
    { name: "create invite, key too short", env: { REFORGED_ADMIN_KEY: "short" }, request: post("/reforged/api/CreateInvite", "{}", { "x-reforged-admin-key": "short" }) },
    { name: "create invite, no uses", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: post("/reforged/api/CreateInvite", '{"uses":0}', host) },
    { name: "create invite, too many uses", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: post("/reforged/api/CreateInvite", '{"uses":1001}', host) },
    { name: "create invite, fractional uses", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: post("/reforged/api/CreateInvite", '{"uses":1.5}', host) },
    { name: "create invite, text uses", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: post("/reforged/api/CreateInvite", '{"uses":"2"}', host) },
    { name: "create invite, name not text", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: post("/reforged/api/CreateInvite", '{"name":5}', host) },
    { name: "invite codes, not the host", request: get("/reforged/api/InviteCodes") },
    { name: "delete invite, not the host", request: { method: "DELETE", path: "/reforged/api/InviteCode/ABCD-EFGH-JKMN" } },
    { name: "delete invite, bad code", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: { method: "DELETE", path: "/reforged/api/InviteCode/a%20b", headers: host } },
    { name: "delete invite, too short", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: { method: "DELETE", path: "/reforged/api/InviteCode/abc", headers: host } },
    { name: "set account status, not the host", request: post("/reforged/api/SetAccountStatus", "{}") },
    { name: "set account status, no status", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: post("/reforged/api/SetAccountStatus", '{"Username":"Fox_9"}', host) },
    { name: "set account status, unknown status", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: post("/reforged/api/SetAccountStatus", '{"Username":"Fox_9","Status":"gone"}', host) },
    { name: "kick, not the host", request: post("/reforged/api/Kick", "{}") },
    { name: "kick, no name", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: post("/reforged/api/Kick", "{}", host) },
    { name: "kick, bad name", env: { REFORGED_ADMIN_KEY: ADMIN_KEY }, request: post("/reforged/api/Kick", '{"Username":"a b"}', host) },
    { name: "server status, limited", env: { REFORGED_SERVER_NAME: "  Fox\u0007hold  ", REFORGED_SERVER_VERSION: "1.2.3", REFORGED_SOURCE_URL: "https://Example.COM/src", REFORGED_SOURCE_COMMIT: " abc123 " }, request: get("/reforged/api/ServerStatus") },
    { name: "server status, defaults and a bad url", env: { REFORGED_SOURCE_URL: "http://example.com/src", APPROVED_EXECUTABLE_SHA256: "xyz" }, request: get("/reforged/api/ServerStatus") },
    { name: "server status, hash and chat", env: { APPROVED_EXECUTABLE_SHA256: "AB".repeat(32), REALTIME_XMPP_ENABLED: "true", REGISTRATION_MODE: "OPEN", REFORGED_SOURCE_URL: "https://user:pw@example.com/" }, request: get("/reforged/api/ServerStatus") },

    // the runtime feed (stable, so no sign-in)
    { name: "runtime manifest", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/client/stable/windows-x86_64`) },
    { name: "runtime manifest, client form", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/stable/windows-x86_64`) },
    { name: "runtime manifest, missing", env: BASE, request: get(`/launcher/v1/runtime/client/stable/windows-x86_64`) },
    { name: "runtime manifest, other platform", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/client/stable/linux`) },
    { name: "runtime manifest, bad target", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/bot/stable/windows-x86_64`) },
    { name: "runtime manifest, server target", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/server/stable/windows-x86_64`) },
    { name: "runtime manifest, bad segment", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/client/stable/win%2Fdows`) },
    { name: "runtime manifest, unreadable", env: BASE, files: { [`${FEED}/latest.json`]: "{not json" }, request: get(`/launcher/v1/runtime/client/stable/windows-x86_64`) },
    { name: "runtime manifest, paused", env: BASE, files: { ...runtimeFiles(), "serving.json": '{"runtimeEnabled":false}' }, request: get(`/launcher/v1/runtime/client/stable/windows-x86_64`) },
    { name: "runtime manifest, switch unreadable", env: BASE, files: { ...runtimeFiles(), "serving.json": "nonsense" }, request: get(`/launcher/v1/runtime/client/stable/windows-x86_64`) },
    { name: "runtime manifest, paused but a bad segment", env: BASE, files: { "serving.json": '{"runtimeEnabled":false}' }, request: get(`/launcher/v1/runtime/client/stable/win%2Fdows`) },
    { name: "runtime download", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/client/stable/windows-x86_64/download`) },
    { name: "runtime download, file missing", env: BASE, files: { [`${FEED}/latest.json`]: manifest() }, request: get(`/launcher/v1/runtime/client/stable/windows-x86_64/download`) },
    { name: "runtime download, a path for a file", env: BASE, files: { ...runtimeFiles(), [`${FEED}/latest.json`]: manifest({ file: "../x.dll" }) }, request: get(`/launcher/v1/runtime/client/stable/windows-x86_64/download`) },
    { name: "runtime download, a path for a version", env: BASE, files: { ...runtimeFiles(), [`${FEED}/latest.json`]: manifest({ version: "../1" }) }, request: get(`/launcher/v1/runtime/client/stable/windows-x86_64/download`) },
    { name: "runtime download, bad target", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/bot/stable/windows-x86_64/download`) },
    { name: "runtime download, the legacy url (shadowed)", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/stable/windows-x86_64/download`) },
    { name: "runtime download, the legacy url with a target as channel", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/client/windows-x86_64/download`) },
    { name: "runtime extra", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/client/stable/windows-x86_64/extra/winmm.dll`) },
    { name: "runtime extra, unknown", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/client/stable/windows-x86_64/extra/other.dll`) },
    { name: "runtime extra, listed but missing", env: BASE, files: { [`${FEED}/latest.json`]: manifest() }, request: get(`/launcher/v1/runtime/client/stable/windows-x86_64/extra/winmm.dll`) },
    { name: "runtime extra, bad target", env: BASE, files: runtimeFiles(), request: get(`/launcher/v1/runtime/bot/stable/windows-x86_64/extra/winmm.dll`) },
    { name: "runtime extra, no manifest", env: BASE, request: get(`/launcher/v1/runtime/client/stable/windows-x86_64/extra/winmm.dll`) },

    // the publisher's gates and switch
    { name: "serving, read", env: PUBLISHER, request: get("/launcher/v1/admin/updates/serving", { "x-update-api-key": "publisher-key" }) },
    { name: "serving, read while paused", env: PUBLISHER, files: { "serving.json": '{"runtimeEnabled":false}' }, request: get("/launcher/v1/admin/updates/serving", { "x-update-api-key": "publisher-key" }) },
    { name: "serving, wrong address", env: { ...PUBLISHER, UPDATE_PUBLISHER_ALLOWED_IPS: "203.0.113.1" }, request: get("/launcher/v1/admin/updates/serving", { "x-update-api-key": "publisher-key" }) },
    { name: "serving, no allowed addresses", env: { UPDATE_PUBLISHER_API_KEY: "publisher-key" }, request: get("/launcher/v1/admin/updates/serving", { "x-update-api-key": "publisher-key" }) },
    { name: "serving, no key", env: PUBLISHER, request: get("/launcher/v1/admin/updates/serving") },
    { name: "serving, wrong key", env: PUBLISHER, request: get("/launcher/v1/admin/updates/serving", { "x-update-api-key": "publisher-kex" }) },
    { name: "serving, key of another length", env: PUBLISHER, request: get("/launcher/v1/admin/updates/serving", { "x-update-api-key": "publisher-key-longer" }) },
    { name: "serving, off", env: PUBLISHER, request: post("/launcher/v1/admin/updates/serving", '{"runtimeEnabled":false}', { "x-update-api-key": "publisher-key" }) },
    { name: "serving, on", env: PUBLISHER, request: post("/launcher/v1/admin/updates/serving", '{"runtimeEnabled":true}', { "x-update-api-key": "publisher-key" }) },
    { name: "serving, not a boolean", env: PUBLISHER, request: post("/launcher/v1/admin/updates/serving", '{"runtimeEnabled":"yes"}', { "x-update-api-key": "publisher-key" }) },
    { name: "serving, no body", env: PUBLISHER, request: post("/launcher/v1/admin/updates/serving", undefined, { "x-update-api-key": "publisher-key" }) },
    { name: "serving, wrong address on the toggle", env: { ...PUBLISHER, UPDATE_PUBLISHER_ALLOWED_IPS: "203.0.113.1" }, request: post("/launcher/v1/admin/updates/serving", '{"runtimeEnabled":false}', { "x-update-api-key": "publisher-key" }) },
    { name: "publish runtime, wrong address", env: { ...PUBLISHER, UPDATE_PUBLISHER_ALLOWED_IPS: "" }, request: post("/launcher/v1/admin/updates/runtime/client/stable/windows-x86_64") },
    { name: "publish runtime, no key", env: PUBLISHER, request: post("/launcher/v1/admin/updates/runtime/client/stable/windows-x86_64") },
    { name: "publish extra, wrong address", env: { ...PUBLISHER, UPDATE_PUBLISHER_ALLOWED_IPS: "" }, request: post("/launcher/v1/admin/updates/runtime/client/stable/windows-x86_64/extra") },
    { name: "publish guard manifest, wrong address", env: { ...PUBLISHER, UPDATE_PUBLISHER_ALLOWED_IPS: "" }, request: post("/launcher/v1/admin/guard-manifests/stable/windows-x86_64", "{}") },
    { name: "publish guard manifest, no key", env: PUBLISHER, request: post("/launcher/v1/admin/guard-manifests/stable/windows-x86_64", "{}") },
    { name: "publish guard manifest, empty envelope", env: { ...PUBLISHER, TARGET_CHANGELIST: "647472" }, request: post("/launcher/v1/admin/guard-manifests/stable/windows-x86_64", "{}", { "x-update-api-key": "publisher-key" }) },
    { name: "publish guard manifest, unknown key", env: { ...PUBLISHER, TARGET_CHANGELIST: "647472" }, request: post("/launcher/v1/admin/guard-manifests/stable/windows-x86_64", '{"schema":1,"keyId":"other","payloadBase64":"e30=","signature":"AAAA"}', { "x-update-api-key": "publisher-key" }) },
    { name: "publish guard manifest, bad signature", env: { ...PUBLISHER, TARGET_CHANGELIST: "647472" }, request: post("/launcher/v1/admin/guard-manifests/stable/windows-x86_64", '{"schema":1,"keyId":"guard-2026-10","payloadBase64":"e30=","signature":"AAAA"}', { "x-update-api-key": "publisher-key" }) },
    { name: "guard manifest, none", env: { TARGET_CHANGELIST: "647472" }, request: get("/launcher/v1/guard-manifests/stable/windows-x86_64") },
    { name: "guard manifest, unreadable", env: { TARGET_CHANGELIST: "647472" }, files: { "guard/stable/windows-x86_64/latest.json": '{"schema":1}' }, request: get("/launcher/v1/guard-manifests/stable/windows-x86_64") },
    { name: "guard manifest, bad segment", env: { TARGET_CHANGELIST: "647472" }, request: get("/launcher/v1/guard-manifests/stable/win%2Fdows") },
    { name: "non-stable channel needs a sign-in", env: BASE, files: runtimeFiles(), request: get("/launcher/v1/runtime/client/beta/windows-x86_64") },
    { name: "non-stable guard manifest needs a sign-in", env: { TARGET_CHANGELIST: "647472" }, request: get("/launcher/v1/guard-manifests/dev/windows-x86_64") },
    { name: "non-stable download needs a sign-in", env: BASE, files: runtimeFiles(), request: get("/launcher/v1/runtime/client/beta/windows-x86_64/download") },

    // the launcher's own feed
    { name: "launcher feed, an update", env: BASE, files: launcherFiles(), request: get("/launcher/v1/updates/windows/x86_64/0.1.0") },
    { name: "launcher feed, up to date", env: BASE, files: launcherFiles(), request: get("/launcher/v1/updates/windows/x86_64/0.2.0") },
    { name: "launcher feed, newer than the server", env: BASE, files: launcherFiles(), request: get("/launcher/v1/updates/windows/x86_64/0.10.0") },
    { name: "launcher feed, an unreadable current version", env: BASE, files: launcherFiles(), request: get("/launcher/v1/updates/windows/x86_64/whatever") },
    { name: "launcher feed, installer missing", env: BASE, files: { "launcher/windows/x86_64/latest.json": launcherManifest() }, request: get("/launcher/v1/updates/windows/x86_64/0.1.0") },
    { name: "launcher feed, no manifest", env: BASE, request: get("/launcher/v1/updates/windows/x86_64/0.1.0") },
    { name: "launcher feed, no signature", env: BASE, files: launcherFiles({ signature: "" }), request: get("/launcher/v1/updates/windows/x86_64/0.1.0") },
    { name: "launcher feed, http url", env: BASE, files: launcherFiles({ url: "http://updates.example.test/launcher/v1/updates/x" }), request: get("/launcher/v1/updates/windows/x86_64/0.1.0") },
    { name: "launcher feed, foreign url", env: BASE, files: launcherFiles({ url: "https://evil.example/launcher/v1/updates/x" }), request: get("/launcher/v1/updates/windows/x86_64/0.1.0") },
    { name: "launcher feed, url with a query", env: BASE, files: launcherFiles({ url: "https://updates.example.test/launcher/v1/updates/x?a=1" }), request: get("/launcher/v1/updates/windows/x86_64/0.1.0") },
    { name: "launcher feed, version not x.y.z", env: BASE, files: launcherFiles({ version: "0.2" }), request: get("/launcher/v1/updates/windows/x86_64/0.1.0") },
    { name: "launcher feed, a release host", env: BASE, files: { "launcher/windows/x86_64/latest.json": launcherManifest({ file: undefined, url: "https://github.com/pranav158/Mystic-Paradox/releases/download/v0.2.0/App.exe" }) }, request: get("/launcher/v1/updates/windows/x86_64/0.1.0") },
    { name: "launcher feed, another release prefix", env: { ...BASE, LAUNCHER_UPDATE_URL_PREFIXES: "https://releases.example.test/dl/, http://no/, bad" }, files: { "launcher/windows/x86_64/latest.json": launcherManifest({ file: undefined, url: "https://releases.example.test/dl/App.exe" }) }, request: get("/launcher/v1/updates/windows/x86_64/0.1.0") },
    { name: "launcher feed, default prefix replaced", env: { ...BASE, LAUNCHER_UPDATE_URL_PREFIXES: "https://releases.example.test/dl/" }, files: { "launcher/windows/x86_64/latest.json": launcherManifest({ file: undefined, url: "https://github.com/pranav158/Mystic-Paradox/releases/download/v0.2.0/App.exe" }) }, request: get("/launcher/v1/updates/windows/x86_64/0.1.0") },
    { name: "launcher feed, bad segment", env: BASE, files: launcherFiles(), request: get("/launcher/v1/updates/win%2Fdows/x86_64/0.1.0") },
    { name: "launcher download", env: BASE, files: launcherFiles(), request: get("/launcher/v1/updates/windows/x86_64/download") },
    { name: "launcher download, installer missing", env: BASE, files: { "launcher/windows/x86_64/latest.json": launcherManifest() }, request: get("/launcher/v1/updates/windows/x86_64/download") },
    { name: "launcher download, no manifest", env: BASE, request: get("/launcher/v1/updates/windows/x86_64/download") },
    { name: "launcher download, redirected", env: BASE, files: { "launcher/windows/x86_64/latest.json": launcherManifest({ file: undefined, url: "https://github.com/pranav158/Mystic-Paradox/releases/download/v0.2.0/App.exe" }) }, request: get("/launcher/v1/updates/windows/x86_64/download") },
    { name: "launcher download, our own url is no redirect", env: BASE, files: { "launcher/windows/x86_64/latest.json": launcherManifest({ file: undefined }) }, request: get("/launcher/v1/updates/windows/x86_64/download") },
    { name: "launcher download, a path for a file", env: BASE, files: { "launcher/windows/x86_64/latest.json": launcherManifest({ file: "../x.exe" }) }, request: get("/launcher/v1/updates/windows/x86_64/download") },
    { name: "launcher installer by version", env: BASE, files: launcherFiles(), request: get("/launcher/v1/updates/windows/x86_64/files/0.2.0/App_0.2.0_x64-setup.exe") },
    { name: "launcher installer, other name", env: BASE, files: launcherFiles(), request: get("/launcher/v1/updates/windows/x86_64/files/0.2.0/evil.exe") },
    { name: "launcher installer, version not in the name", env: BASE, files: launcherFiles(), request: get("/launcher/v1/updates/windows/x86_64/files/0.3.0/App_0.2.0_x64-setup.exe") },
    { name: "launcher installer, not there", env: BASE, files: launcherFiles(), request: get("/launcher/v1/updates/windows/x86_64/files/0.2.0/Other_0.2.0_x64-setup.exe") },

    // the sync script's reads of the tester logs
    { name: "launcher logs, no token", env: SYNC, files: logFiles, request: get("/admin/v1/launcher-logs") },
    { name: "launcher logs, wrong token", env: SYNC, files: logFiles, request: get("/admin/v1/launcher-logs", { "x-sync-token": "nope" }) },
    { name: "launcher logs, none configured", files: logFiles, request: get("/admin/v1/launcher-logs", sync) },
    { name: "launcher logs, list", env: SYNC, files: logFiles, request: get("/admin/v1/launcher-logs", sync) },
    { name: "launcher logs, list since", env: SYNC, files: logFiles, request: get("/admin/v1/launcher-logs?since=2026-10-01T12:00:00Z", sync) },
    { name: "launcher logs, list since the last", env: SYNC, files: logFiles, request: get("/admin/v1/launcher-logs?since=2026-10-02T00:00:00.000Z", sync) },
    { name: "launcher logs, list since garbage", env: SYNC, files: logFiles, request: get("/admin/v1/launcher-logs?since=garbage", sync) },
    { name: "launcher logs, no root", env: SYNC, request: get("/admin/v1/launcher-logs", sync) },
    { name: "launcher logs, a file", env: SYNC, files: logFiles, request: get(`/admin/v1/launcher-logs/u1/${SESSION}/launcher.log`, sync) },
    { name: "launcher logs, a missing file", env: SYNC, files: logFiles, request: get(`/admin/v1/launcher-logs/u1/${SESSION}/nothing.log`, sync) },
    { name: "launcher logs, a bad path", env: SYNC, files: logFiles, request: get(`/admin/v1/launcher-logs/u1/${SESSION}/..%2Fx`, sync) },
    { name: "launcher log upload, no sign-in", env: SYNC, request: { method: "PUT", path: `/launcher/v1/logs/sessions/${SESSION}/launcher.log` } },
];

// ---------------------------------------------------------------- the harness

const SEEN_HEADERS = ["content-type", "cache-control", "content-length", "location", "content-disposition", "vary", "retry-after"];
const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g;
const ENV_KEYS = [
    "LAUNCHER_UPDATE_ROOT", "LAUNCHER_LOG_STORAGE_DIR", "UPDATE_PUBLIC_BASE_URL", "UPDATE_PUBLISHER_ALLOWED_IPS", "UPDATE_PUBLISHER_API_KEY",
    "LAUNCHER_UPDATE_URL_PREFIXES", "LAUNCHER_LOG_SYNC_TOKEN", "NODE_ENV", "RUSTIC_ISLES_METRICS_TOKEN", "RUSTIC_ISLES_ADMIN_API",
    "REGISTRATION_MODE", "REFORGED_ADMIN_KEY", "REFORGED_SERVER_NAME", "REFORGED_SERVER_VERSION", "REFORGED_SOURCE_URL", "REFORGED_SOURCE_COMMIT",
    "APPROVED_EXECUTABLE_SHA256", "REALTIME_XMPP_ENABLED", "TARGET_CHANGELIST", "MYSTPAX_OLD", "MYSTPAX_AGAIN", "RUSTIC_ISLES_GUARD_MODE",
    "RUSTIC_ISLES_REGISTRATION_APPROVAL", "RUSTIC_ISLES_REGISTRATION_AUTO_APPROVE", "RUSTIC_ISLES_TLS_TERMINATED_BY_PROXY", "BIND_HOST",
    "REFORGED_HTTP_ONLY", "DEPLOYSERVER_URL", "PARADOX_GAME_DATA_DIR", "RUSTIC_ISLES_SERVICE_ROLE", "RUNTIME_EXTRA_FILES"
];

// The modules read signing keys when loaded; none of the captured routes uses them
process.env.AUTH_SIGNING_PRIVKEY_B64 = Buffer.from("not a key").toString("base64");
process.env.AUTH_SIGNING_PUBKEY_B64 = process.env.AUTH_SIGNING_PRIVKEY_B64;
process.env.API_KEY_HASH_SECRET = "a-test-secret-of-at-least-32-characters";

const nodeRequire = createRequire(import.meta.url);
process.env.RUSTIC_ISLES_P2P_MODULE = "off";

const express = (await import(pathToFileURL(path.join(workRoot, "node_modules/express/index.js")).href)).default;
const quiet = () => { for (const method of ["log", "info", "warn", "error"] as const) (console as any)[method] = () => {}; };
const realLog = console.log.bind(console);

async function run(spec: Case, serial: number) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "golden-"));
    for (const [name, text] of Object.entries(spec.files ?? {})) {
        const target = path.join(dir, name);
        fs.mkdirSync(path.dirname(target), { recursive: true });
        fs.writeFileSync(target, text);
    }
    for (const key of ENV_KEYS) delete process.env[key];
    // Files under `logs/` are the log root, the rest the update root
    process.env.LAUNCHER_UPDATE_ROOT = dir;
    process.env.LAUNCHER_LOG_STORAGE_DIR = path.join(dir, "logs");
    Object.assign(process.env, spec.env ?? {});
    // tsx runs the sources as CommonJS: forgetting them is what makes a router read its settings afresh
    for (const key of Object.keys(nodeRequire.cache)) if (key.includes(`${path.sep}src${path.sep}`) && !key.includes("node_modules")) delete nodeRequire.cache[key];
    const load = async (file: string) => nodeRequire(path.join(workRoot, "src", file));
    const app = express();
    app.set("trust proxy", "loopback");
    app.use(express.json());
    app.use(express.urlencoded({ extended: true }));
    if (process.env.RUSTIC_ISLES_ADMIN_API === "on") app.use("/", (await load("routes/releaseGates.ts")).releaseGatesRouter);
    app.use("/", (await load("routes/metrics.ts")).metricsRouter);
    app.use("/", (await load("routes/loginQueue.ts")).loginQueueRouter);
    app.use("/", (await load("routes/reforgedApi.ts")).reforgedApiRouter);
    app.use("/", (await load("routes/launcherUpdates.ts")).launcherUpdatesRouter);
    app.use("/", (await load("routes/launcherLogs.ts")).launcherLogsRouter);
    app.use("/", (await load("routes/adminLauncherLogs.ts")).adminLauncherLogsRouter);
    app.use((_req: any, res: any) => { res.status(404); res.send(); });
    const server = http.createServer(app);
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const { port } = server.address() as any;
    try {
        const { method, path: urlPath, headers, json } = spec.request;
        const init: RequestInit = { method, headers: { ...(headers ?? {}), ...(json === undefined ? {} : { "content-type": "application/json" }) }, redirect: "manual" };
        if (json !== undefined) init.body = json;
        const answer = await fetch(`http://127.0.0.1:${port}${urlPath}`, init);
        const body = (await answer.text()).replace(UUID, "<id>");
        const seen: Record<string, string> = {};
        for (const name of SEEN_HEADERS) {
            const value = answer.headers.get(name);
            if (value !== null) seen[name] = value.replace(UUID, "<id>");
        }
        let after: Record<string, string> | undefined;
        if (spec.request.method === "POST" && fs.existsSync(path.join(dir, "serving.json"))) {
            after = { "serving.json": JSON.parse(fs.readFileSync(path.join(dir, "serving.json"), "utf8")).runtimeEnabled };
        }
        return {
            name: spec.name,
            env: spec.env ?? {},
            files: spec.files ?? {},
            request: spec.request,
            response: { status: answer.status, headers: seen, body: spec.skipBody ? null : body },
            ...(after === undefined ? {} : { servingAfter: after["serving.json"] }),
        };
    } finally {
        await new Promise((resolve) => server.close(resolve));
        fs.rmSync(dir, { recursive: true, force: true });
    }
}

quiet();
const out = [];
let serial = 0;
for (const spec of CASES) out.push(await run(spec, serial++));
fs.writeFileSync(path.join(outDir, "reforged_api.json"), `${JSON.stringify(out, null, 2)}\n`);
realLog(`${out.length} cases`);
process.exit(0);
