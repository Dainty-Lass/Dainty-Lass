/*
 * Copyright (C) 2026 DaintyLass contributors
 * SPDX-License-Identifier: AGPL-3.0-only
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
    ADMIN_KEY_HEADER,
    AssetLeaf,
    ClassifyGameData,
    GAME_DATA_FILES,
    GATEWAY_HEADER,
    InstanceTitle,
    InviteNoteFrom,
    IsInviteCode,
    IsLocalAdminCall,
    LooksLikeAccountKey,
    MakeAccountKey,
    MakeInviteCode,
    NormalizeUsername,
    RegistrationModeFrom,
    StatusInstancesFrom
} from "./launcherApiRules";

test("a username is 3-16 letters, digits and underscores, trimmed", () => {
    assert.equal(NormalizeUsername("Slayer_01"), "Slayer_01");
    assert.equal(NormalizeUsername("  Fox  "), "Fox");
    assert.equal(NormalizeUsername("ab"), undefined);
    assert.equal(NormalizeUsername("x".repeat(17)), undefined);
    assert.equal(NormalizeUsername("two words"), undefined);
    assert.equal(NormalizeUsername("açaí"), undefined);
    assert.equal(NormalizeUsername(7), undefined);
});

test("invite codes: three groups of four from the look-alike-free alphabet", () => {
    for (let Round = 0; Round < 40; Round++) {
        const Code = MakeInviteCode();
        assert.match(Code, /^[A-HJ-KM-NP-Z2-9]{4}-[A-HJ-KM-NP-Z2-9]{4}-[A-HJ-KM-NP-Z2-9]{4}$/);
        assert.ok(IsInviteCode(Code));
    }
    assert.equal(MakeInviteCode(() => 0), "AAAA-AAAA-AAAA");
    assert.equal(MakeInviteCode(() => 30), "9999-9999-9999");
    assert.ok(!IsInviteCode("abc"));
    assert.ok(!IsInviteCode("no spaces"));
    assert.ok(!IsInviteCode(1234));
});

test("account keys are RRK_ and 48 hex digits, and look-alikes are at least worth a lookup", () => {
    const Key = MakeAccountKey();
    assert.match(Key, /^RRK_[0-9a-f]{48}$/);
    assert.ok(LooksLikeAccountKey(Key));
    assert.ok(LooksLikeAccountKey("UUK_somebody_elses_key"));
    assert.ok(!LooksLikeAccountKey("tiny"));
    assert.ok(!LooksLikeAccountKey("has a space"));
    assert.ok(!LooksLikeAccountKey('quote"inside'));
});

test("the registration mode is invite codes unless told OPEN or NONE", () => {
    assert.equal(RegistrationModeFrom(undefined), "INVITECODE");
    assert.equal(RegistrationModeFrom(" open "), "OPEN");
    assert.equal(RegistrationModeFrom("None"), "NONE");
    assert.equal(RegistrationModeFrom("whatever"), "INVITECODE");
});

test("an invite note loses control characters and is capped at 64", () => {
    assert.equal(InviteNoteFrom("for\tAlex\u0000"), "for Alex");
    assert.equal(InviteNoteFrom("   "), undefined);
    assert.equal(InviteNoteFrom("y".repeat(80))?.length, 64);
    assert.equal(InviteNoteFrom(null), undefined);
});

test("admin calls: loopback, not forwarded, the right key, and a usable key configured", () => {
    const Key = "k".repeat(40);
    const Call = (headers: Record<string, string>, address = "127.0.0.1") => ({ headers, socket: { remoteAddress: address } }) as never;
    assert.ok(IsLocalAdminCall(Call({ [ADMIN_KEY_HEADER]: Key }), Key));
    assert.ok(IsLocalAdminCall(Call({ [ADMIN_KEY_HEADER]: Key }, "::ffff:127.0.0.1"), Key));
    assert.ok(IsLocalAdminCall(Call({ [ADMIN_KEY_HEADER]: Key }, "::1"), Key));
    assert.ok(!IsLocalAdminCall(Call({ [ADMIN_KEY_HEADER]: Key }, "10.0.0.2"), Key));
    assert.ok(!IsLocalAdminCall(Call({ [ADMIN_KEY_HEADER]: Key, [GATEWAY_HEADER]: "1" }), Key));
    assert.ok(!IsLocalAdminCall(Call({ [ADMIN_KEY_HEADER]: Key, "x-forwarded-for": "1.2.3.4" }), Key));
    assert.ok(!IsLocalAdminCall(Call({ [ADMIN_KEY_HEADER]: "k".repeat(39) + "x" }), Key));
    assert.ok(!IsLocalAdminCall(Call({}), Key));
    assert.ok(!IsLocalAdminCall(Call({ [ADMIN_KEY_HEADER]: "short" }), "short"));
    assert.ok(!IsLocalAdminCall(Call({ [ADMIN_KEY_HEADER]: Key }), undefined));
});

test("the Director's instances are read field by field, and hunts say who is in them", () => {
    const Raw = [
        { id: "city-8770", kind: "city", map: "/Game/Maps/Ramsgate/Ramsgate_P", players: 3, maxPlayers: 0, startedAt: "2026-10-26T10:00:00Z" },
        { id: "hunt-8771", kind: "hunt", map: "/Game/Maps/islands/x/x_persistent", behemoth: "/Game/Behemoths/BP_Skarn.BP_Skarn_C", players: 2, maxPlayers: 4, playerIds: ["a", "b", 3], startedAt: "not a date" },
        { id: "x", kind: "spaceship" },
        "junk",
        null
    ];
    const { instances, huntOf } = StatusInstancesFrom(Raw);
    assert.equal(instances.length, 2);
    assert.deepEqual(instances[0], { id: "city-8770", kind: "city", title: "Ramsgate", map: "Ramsgate_P", behemoth: null, players: 3, maxPlayers: 0, startedAt: "2026-10-26T10:00:00.000Z" });
    assert.deepEqual(instances[1], { id: "hunt-8771", kind: "hunt", title: "Skarn", map: "x_persistent", behemoth: "BP_Skarn", players: 2, maxPlayers: 4, startedAt: null });
    assert.deepEqual([...huntOf.entries()], [["a", "hunt-8771"], ["b", "hunt-8771"]]);
    assert.deepEqual(StatusInstancesFrom("nope").instances, []);
    assert.deepEqual(StatusInstancesFrom({ length: 1 }).instances, []);
});

test("asset leaves and instance titles", () => {
    assert.equal(AssetLeaf("/Game/Behemoths/Skarn.Skarn_C"), "Skarn");
    assert.equal(AssetLeaf("bare"), "bare");
    assert.equal(InstanceTitle("dojo", null, ""), "Training Grounds");
    assert.equal(InstanceTitle("tutorial", null, ""), "Tutorial");
    assert.equal(InstanceTitle("hunt", "/Game/B/BPC_Drask_Heroic.BPC_Drask_Heroic_C", ""), "Drask Heroic");
    assert.equal(InstanceTitle("hunt", null, "/Game/Maps/islands/frost_bay/frost_bay_persistent"), "frost bay");
    assert.equal(InstanceTitle("hunt", null, ""), "Hunt");
});

test("game data is real, synthetic or missing", () => {
    const Dir = fs.mkdtempSync(path.join(os.tmpdir(), "dl-gamedata-"));
    try {
        assert.equal(ClassifyGameData(Dir, GAME_DATA_FILES.slice(0, 2)), "missing");
        fs.writeFileSync(path.join(Dir, "a.json"), "{}");
        fs.writeFileSync(path.join(Dir, "b.json"), "{\"real\":1}");
        assert.equal(ClassifyGameData(Dir, ["a.json", "b.json"]), "real");
        fs.writeFileSync(path.join(Dir, "a.example.json"), "{}");
        assert.equal(ClassifyGameData(Dir, ["a.json", "b.json"]), "synthetic");
        assert.equal(ClassifyGameData(Dir, ["a.json", "c.json"]), "missing");
    } finally {
        fs.rmSync(Dir, { recursive: true, force: true });
    }
});
