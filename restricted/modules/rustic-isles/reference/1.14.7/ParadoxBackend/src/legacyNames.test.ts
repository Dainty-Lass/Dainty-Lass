import assert from "node:assert/strict";
import test from "node:test";
import type { NextFunction, Request, Response } from "express";
import { AcceptLegacyEnv, AcceptLegacyHeaders, CanonicalArtifactName } from "./legacyNames";
import { isAllowedLauncherLogFileName } from "./services/launcherLogNames";

// The names from before the Rustic Isles rename stay accepted for one release.

test("an old env name is read under the new one, and the new one wins", () => {
    const env: NodeJS.ProcessEnv = {
        MYSTICPARADOX_METRICS_TOKEN: "old",
        MYSTICPARADOX_GUARD_MODE: "observe",
        RUSTIC_ISLES_GUARD_MODE: "enforce",
    };
    AcceptLegacyEnv(env);
    assert.equal(env.RUSTIC_ISLES_METRICS_TOKEN, "old");
    assert.equal(env.RUSTIC_ISLES_GUARD_MODE, "enforce");
    assert.equal(env.MYSTICPARADOX_METRICS_TOKEN, "old", "kept for older runtimes");
});

test("an old header is read under the new one, and the new one wins", () => {
    const req = { headers: {
        "x-mysticparadox-gameserver-apikey": "old",
        "x-mysticparadox-request-id": "old-request",
        "x-rustic-isles-request-id": "new-request",
    } } as unknown as Request;
    let called = false;
    AcceptLegacyHeaders(req, {} as Response, (() => { called = true; }) as NextFunction);
    assert.ok(called);
    assert.equal(req.headers["x-rustic-isles-gameserver-apikey"], "old");
    assert.equal(req.headers["x-rustic-isles-request-id"], "new-request");
});

test("the old runtime DLL and log names are still recognised", () => {
    assert.equal(CanonicalArtifactName("MysticParadox.dll"), "Rustic-Isles.dll");
    assert.equal(CanonicalArtifactName("mysticparadox.DLL"), "Rustic-Isles.dll");
    assert.equal(CanonicalArtifactName("winmm.dll"), "winmm.dll");
    assert.ok(isAllowedLauncherLogFileName("runtime-mysticparadox_dll_port7777.log"));
});
