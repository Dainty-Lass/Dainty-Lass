import assert from "node:assert/strict";
import test from "node:test";
import { AcceptLegacyEnv, LegacyEnvName } from "./legacyNames";

// The names from before the Rustic Isles rename stay accepted for one release.

test("an old env name is read under the new one, and a game server still gets the old one", () => {
    const env: NodeJS.ProcessEnv = { MYSTICPARADOX_TRIALS_WEEK: "003", RUSTIC_ISLES_GAMESERVER_LAUNCH_ID: "new" };
    AcceptLegacyEnv(env);
    assert.equal(env.RUSTIC_ISLES_TRIALS_WEEK, "003");
    assert.equal(LegacyEnvName("RUSTIC_ISLES_GAMESERVER_LAUNCH_ID"), "MYSTICPARADOX_GAMESERVER_LAUNCH_ID");
    assert.equal(LegacyEnvName("PORT"), undefined);
});
