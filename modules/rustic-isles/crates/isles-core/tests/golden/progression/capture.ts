/*
 * Records the Node answers the Rust progression area must match (RU5 progression, golden tests).
 *
 * Runs the real routers of reference/1.14.7/ParadoxBackend (routes/system.ts and routes/progression.ts with their
 * controllers) against in-memory repositories, the way the Node tests do (routes/systemHostWrites.gamedata.test.ts), and
 * records each request with the status and the exact body text it was answered with. The Rust test replays the same
 * requests against its handlers on SQLite and compares the texts, so field order and number formatting are checked, not
 * just values. Dates and minted grant ids are normalised on both sides.
 *
 *   docker run --rm -v <repo>:/repo node:22 sh /repo/crates/isles-core/tests/golden/progression/record.sh
 */

import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import type { AddressInfo } from "node:net";
import type { Server } from "node:http";

const Fixture = process.env.GOLDEN_FIXTURE_DIR ?? path.resolve(__dirname, "..", "..", "..", "tests", "golden", "progression");
process.env.PARADOX_GAME_DATA_DIR = path.join(Fixture, "game-data");
process.env.RUSTIC_ISLES_P2P_MODULE = "off";
delete process.env.GAMESERVER_NO_PLAYER_USER_ID;
delete process.env.DEV_USER_ID;

import express from "express";

type Json = any;

// ---- in-memory repositories (the semantics of persistence/sqlite/repositories/*) ----------------------------------------------------

const Tracks = new Map<string, any>();
const Objectives = new Map<string, any>();
const Grants = new Map<string, any>();
const Escalations = new Map<string, any>();
const Journeys = new Map<string, any>();
const Crumbs = new Map<string, any>();
const Encountered = new Map<string, any>();
const Characters = new Map<string, string>();   // characterId -> userId
const Users = new Set<string>();

const Max = (current: unknown, value: number) => (typeof current === "number" && current >= value ? current : value);

function TrackBase(userId: string, progressionId: string, insertDefaults: Json, now: string) {
    const Existing = Tracks.get(`${userId}::${progressionId}`);
    return Existing ? { ...Existing } : { userId, progressionId, confirmedFremiumRank: 0, confirmedPremiumRank: 0, createdAt: now, ...insertDefaults };
}

const TrackRepository = {
    getAllForUser: async (userId: string) => [...Tracks.values()].filter((Row) => Row.userId === userId).map((Row) => ({ ...Row })),
    get: async (userId: string, progressionId: string) => {
        const Row = Tracks.get(`${userId}::${progressionId}`);
        return Row === undefined ? undefined : { ...Row };
    },
    increment: async (userId: string, progressionId: string, amount: number) => {
        const Now = new Date().toISOString();
        const Doc = TrackBase(userId, progressionId, {}, Now);
        Doc.progress = (typeof Doc.progress === "number" ? Doc.progress : 0) + amount;
        Doc.updateVersion = (Doc.updateVersion ?? 0) + 1;
        Doc.updatedAt = Now;
        Tracks.set(`${userId}::${progressionId}`, Doc);
        return { ...Doc };
    },
    setProgressIfGreater: async (userId: string, progressionId: string, value: number) => {
        const Now = new Date().toISOString();
        const Doc = TrackBase(userId, progressionId, {}, Now);
        Doc.progress = Max(Doc.progress, value);
        Doc.updateVersion = (Doc.updateVersion ?? 0) + 1;
        Doc.updatedAt = Now;
        Tracks.set(`${userId}::${progressionId}`, Doc);
        return { ...Doc };
    },
    setConfirmedFremiumRank: async (userId: string, progressionId: string, rank: number) => {
        const Now = new Date().toISOString();
        const Doc = TrackBase(userId, progressionId, { progress: 0 }, Now);
        Doc.confirmedFremiumRank = Max(Doc.confirmedFremiumRank, rank);
        Doc.updateVersion = (Doc.updateVersion ?? 0) + 1;
        Doc.updatedAt = Now;
        Tracks.set(`${userId}::${progressionId}`, Doc);
        return { ...Doc };
    },
    appendObjectiveEvent: async () => undefined,
    getAllObjectivesForUser: async (userId: string) => [...Objectives.values()].filter((Row) => Row.userId === userId).map((Row) => ({ ...Row })),
    getObjectives: async (userId: string, Ids: string[]) => Ids.map((Id) => Objectives.get(`${userId}::${Id}`)).filter((Row) => Row !== undefined).map((Row) => ({ ...Row })),
    mergeObjectives: async (userId: string, Rows: { objectiveId: string; value: number; completedCount: number }[]) => {
        const Now = new Date().toISOString();
        for (const Row of Rows) {
            const Before = Objectives.get(`${userId}::${Row.objectiveId}`);
            Objectives.set(`${userId}::${Row.objectiveId}`, {
                userId, objectiveId: Row.objectiveId, value: Max(Before?.value, Row.value), completedCount: Max(Before?.completedCount, Row.completedCount), updatedAt: Now
            });
        }
    },
    updateReforge: async (userId: string, progressionId: string, reforgeId: string, expected: any, set: any) => {
        const Row = Tracks.get(`${userId}::${progressionId}`);
        const Current = Row?.reforge;
        if (Row === undefined || Current == undefined || Current.reforgeId !== reforgeId) return false;
        for (const [Key, Value] of Object.entries(expected)) if (Current[Key] !== Value) return false;
        let Modified = false;
        const Next = { ...Current };
        for (const [Key, Value] of Object.entries(set)) {
            if (!(Key in Current) || JSON.stringify(Current[Key]) !== JSON.stringify(Value)) Modified = true;
            Next[Key] = Value;
        }
        if (!Modified) return false;
        Row.reforge = Next;
        return true;
    },
};

const GrantRepository = {
    findByGrantId: async (GrantId: string) => Grants.get(GrantId),
    insertApplied: async (Record: any) => { Grants.set(Record.grantId, Record); },
};

const EscalationRepository = {
    get: async (userId: string, seasonId: string) => Escalations.get(`${userId}::${seasonId}`),
    saveIfVersion: async (record: any, expected: number | undefined) => {
        const Key = `${record.userId}::${record.seasonId}`;
        const Held = Escalations.get(Key);
        if (expected === undefined ? Held !== undefined : Held?.updateVersion !== expected) return undefined;
        Escalations.set(Key, record);
        return record;
    },
};

const JourneyRepository = {
    findByUserId: async (userId: string) => {
        const Row = Journeys.get(userId);
        return Row === undefined ? undefined : { ...Row, revision: Row.revision ?? 0 };
    },
    createIfAbsent: async (record: any) => {
        if (Journeys.has(record.userId)) return false;
        Journeys.set(record.userId, { ...record });
        return true;
    },
    updateIfRevision: async (userId: string, nodesJson: string, updateVersion: number, expectedRevision: number) => {
        const Row = Journeys.get(userId);
        if (Row === undefined || (Row.revision ?? 0) !== expectedRevision) return false;
        Row.nodes = nodesJson;
        Row.updateVersion = updateVersion;
        Row.revision = expectedRevision + 1;
        return true;
    },
};

const CrumbRepository = {
    findByCharacterIdAndUserId: async (characterId: string, userId: string) => {
        const Row = Crumbs.get(characterId);
        return Row?.userId === userId ? { ...Row } : undefined;
    },
    create: async (record: any) => { Crumbs.set(record.characterId, { ...record }); },
    update: async (characterId: string, userId: string, json: string, updateVersion: number) => {
        const Row = Crumbs.get(characterId);
        if (Row?.userId === userId) { Row.breadcrumbs = json; Row.updateVersion = updateVersion; }
    },
};

const EncounteredRepository = {
    findByCharacterIdAndUserId: async (characterId: string, userId: string) => {
        const Row = Encountered.get(characterId);
        return Row?.userId === userId ? { ...Row } : undefined;
    },
    create: async (record: any) => { Encountered.set(record.characterId, { ...record }); },
    updateContent: async (characterId: string, userId: string, json: string) => {
        const Row = Encountered.get(characterId);
        if (Row?.userId === userId) Row.encounteredcontent = json;
    },
};

const CharacterRepository = {
    findByCharacterIdAndUserId: async (characterId: string, userId: string) =>
        Characters.get(characterId) === userId ? { characterId, userId } : undefined,
    findManyByUserId: async () => [],
};

const Repositories = {
    progressionTracks: TrackRepository,
    progressionGrants: GrantRepository,
    escalationProgress: EscalationRepository,
    playerJourney: JourneyRepository,
    breadcrumbs: CrumbRepository,
    encounteredContent: EncounteredRepository,
    characters: CharacterRepository,
};

// ---- the host hooks, as a recorder ---------------------------------------------------------------------------------------------------

const Hooks = {
    authorized: [] as [string, string][],
    records: [] as { kind: string; target: string; summary: any }[],
    denial: undefined as any,
    probeRefuses: [] as string[],
    policyRefuses: true,
    notes: [] as string[],
};

const DENIAL = { status: 403, body: { code: "P2P_HOST_WRITE_DENIED", message: "This host may not write this player's state.", payload: null } };

function Replace(modulePath: string, overrides: Record<string, unknown>): void {
    const Resolved = require.resolve(modulePath);
    const Replacement = Object.create(require(modulePath));
    for (const [Name, Value] of Object.entries(overrides)) Object.defineProperty(Replacement, Name, { value: Value, enumerable: true });
    require.cache[Resolved]!.exports = Replacement;
}

function Fake(modulePath: string, exports: Record<string, unknown>): void {
    const Resolved = require.resolve(modulePath);
    require.cache[Resolved] = { id: Resolved, filename: Resolved, loaded: true, exports, children: [], paths: [] } as unknown as NodeJS.Module;
}

const Quiet = () => undefined;
Fake("./logger", { logger: { level: "silent", trace: Quiet, debug: Quiet, info: Quiet, warn: Quiet, error: Quiet, fatal: Quiet, child: () => ({}) }, ApplyDiagnosticsLogProfile: Quiet });
Replace("./persistence", {
    GetRepositories: () => Repositories,
    GetUnitOfWork: () => ({ withTransaction: async (Action: any) => Action(Repositories, {}) }),
});
Replace("./extensions/p2p", {
    GetP2PExtension: () => ({
        hostWrites: {
            authorize: async (_req: unknown, kind: string, target: string, options?: any) => {
                Hooks.authorized.push([kind, target]);
                if (options?.probe && Hooks.probeRefuses.includes(kind) && Hooks.denial === undefined) return DENIAL;
                return Hooks.denial;
            },
            record: async (_req: unknown, write: { kind: string; targetUserId: string; summary: any }) => {
                Hooks.records.push({ kind: write.kind, target: write.targetUserId, summary: write.summary });
            },
        },
        hostPolicy: {
            refuses: () => Hooks.policyRefuses,
            noteWouldRefuse: (what: string, detail: string) => { Hooks.notes.push(`${what} ${detail}`); },
        },
    }),
});
Fake("./activity/activityMonitor", {
    RecordXpActivity: Quiet, RecordReforgeActivity: Quiet, RecordSlayersPathUnlocksActivity: async () => undefined,
    RecordChallengeClaimsActivity: Quiet,
});
Fake("./activity/activityRules", { IsIgnoredActivitySource: () => false });
Fake("./controllers/character", { GetCharactersForUid: async () => [] });
Fake("./controllers/inventory", { GetInventoryForUserIdAndCharacterId: async () => undefined, RunInventoryTransaction: async () => false });
Fake("./controllers/wallet", { IsPrestigeSourceTrack: () => false });
Fake("./middleware/HasParadoxBackendAuth", {
    HasParadoxBackendAuth: (req: any, res: any, next: () => void) => {
        const Role = req.headers["x-test-role"];
        const User = req.headers["x-test-user"];
        if (Role === "gs") req.AuthData = { IsGameserver: true, ...(User ? { userId: User } : {}) };
        else if (Role === "player") req.AuthData = { userId: User };
        else if (Role === "host") {
            const Roster = String(req.headers["x-test-roster"] ?? "").split(",").filter((Id) => Id.length > 0);
            req.AuthData = { IsPlayerHostRuntime: true, IsGameserver: false, userId: User, RuntimeRosterAccountIds: Roster };
        }
        else { res.status(401).send(); return; }
        next();
    },
});

// ---- the scenarios ------------------------------------------------------------------------------------------------------------------------

type Step = {
    name: string;
    op?: string;
    [key: string]: any;
    method?: string;
    path?: string;
    role?: string;
    user?: string;
    roster?: string;
    headers?: Record<string, string>;
    body?: Json;
};

const GS = "gs";
const Req = (name: string, method: string, path: string, role: string | undefined, user: string | undefined, body?: Json, headers: Record<string, string> = {}, extra: any = {}): Step =>
    ({ name, method, path, role, user, body, headers, ...extra });
const Grant = (name: string, user: string, track: string, amount: string | number, id: string | undefined, role = GS, extra: any = {}) =>
    Req(name, "POST", `/progression/${user}/${track}/${amount}`, role, undefined, undefined, id === undefined ? {} : { "x-rustic-isles-grant-id": id }, extra);

const Node = (id: string, status: number, extra: any = {}) => ({ node_id: id, node_status: status, objectives: [], ...extra });

const Steps: Step[] = [
    // hunt pass, lists and reads
    Req("huntpass", "GET", "/huntpass/player-1", "player", "player-1"),
    Req("progression list, nothing stored", "GET", "/progression/player-1", "player", "player-1"),
    Req("progression list for the no-player sentinel", "GET", "/progression/INVALID", GS, undefined),
    Req("progression list of another account", "GET", "/progression/player-2", "player", "player-1"),
    Req("progression list, no auth", "GET", "/progression/player-1", undefined, undefined),
    Req("single track, nothing stored", "GET", "/progression/player-1/ExperienceTrack_Weapon_Axe", "player", "player-1"),
    Req("single track, reserved segment", "GET", "/progression/config/x", "player", "player-1"),
    Req("single track, sentinel", "GET", "/progression/INVALID/ExperienceTrack_Weapon_Axe", GS, undefined),
    Req("objectives, none", "GET", "/progression/objectives/player-1", "player", "player-1"),
    Req("objectives, sentinel", "GET", "/progression/objectives/INVALID", GS, undefined),
    Req("weapon tracker", "GET", "/weapon_tracker/player-1/RHLDQHG6UNBG3KROD7MWHKZSKU", "player", "player-1"),
    Req("weapon tracker, other account", "GET", "/weapon_tracker/player-2/X", "player", "player-1"),
    Req("tracked objectives", "GET", "/progression/tracked_objectives/player-1", "player", "player-1"),
    Req("tracked objectives post", "POST", "/progression/tracked_objectives/player-1", "player", "player-1", { current_set: "x" }),

    // XP grants
    Grant("grant", "player-1", "ExperienceTrack_Weapon_Axe", 250, "g1"),
    Grant("grant replay", "player-1", "ExperienceTrack_Weapon_Axe", 250, "g1"),
    Grant("grant id reused with another amount", "player-1", "ExperienceTrack_Weapon_Axe", 251, "g1"),
    Grant("grant id reused for another user", "player-2", "ExperienceTrack_Weapon_Axe", 250, "g1"),
    Grant("grant without an id from a game server", "player-1", "ExperienceTrack_PlayerLevel", 98, undefined),
    Grant("grant without an id from a host-less player", "player-1", "ExperienceTrack_PlayerLevel", 98, undefined, "player", { user: "player-1" }),
    Grant("grant with a bad id", "player-1", "ExperienceTrack_PlayerLevel", 98, "-bad id"),
    Req("grant with header and body ids that differ", "POST", "/progression/player-1/ExperienceTrack_PlayerLevel/5", GS, undefined, { grant_id: "g-body" }, { "x-rustic-isles-grant-id": "g-head" }),
    Req("grant with the id in the body", "POST", "/progression/player-1/ExperienceTrack_PlayerLevel/5", GS, undefined, { grantId: "g-body-2" }),
    Req("grant with an idempotency key", "POST", "/progression/player-1/ExperienceTrack_PlayerLevel/6", GS, undefined, undefined, { "idempotency-key": "g-key" }),
    Grant("grant, amount zero", "player-1", "ExperienceTrack_PlayerLevel", 0, "g2"),
    Grant("grant, amount text", "player-1", "ExperienceTrack_PlayerLevel", "abc", "g3"),
    Grant("grant, amount fraction", "player-1", "ExperienceTrack_PlayerLevel", "1.5", "g4"),
    Grant("grant, amount exponent", "player-1", "ExperienceTrack_PlayerLevel", "1e2", "g5"),
    Grant("grant, amount over the maximum", "player-1", "ExperienceTrack_PlayerLevel", 100001, "g6"),
    Grant("grant, amount at the maximum", "player-1", "ExperienceTrack_Weapon_Sword", 100000, "g7"),
    Grant("grant, sentinel user", "INVALID", "ExperienceTrack_PlayerLevel", 5, "g8"),
    Grant("grant, unapproved track", "player-1", "Linked_Slayer_Slot_1", 5, "g9"),
    Grant("grant, unknown track", "player-1", "ExperienceTrack_Nothing", 5, "g10"),
    Grant("grant, long track", "player-1", "x".repeat(129), 5, "g11"),
    Grant("grant to the active hunt pass", "player-1", "season43", 20, "g12"),
    Grant("grant to the other hunt pass", "player-1", "season19", 20, "g13"),
    Grant("grant to a mastery track", "player-1", "MasteryTrack_PlayerLevel", 2, "g14"),
    Grant("grant by a player", "player-1", "ExperienceTrack_PlayerLevel", 5, "g15", "player", { user: "player-1" }),
    Grant("grant, prestige without a reforge", "player-1", "PrestigeTrack_Weapon_Axe", 1, "g16"),
    Req("progression list after grants", "GET", "/progression/player-1", "player", "player-1"),
    Req("single track after grants", "GET", "/progression/player-1/ExperienceTrack_Weapon_Axe", "player", "player-1"),
    Req("single track, a stored track outside the defaults", "GET", "/progression/player-1/season19", GS, undefined),

    // rank confirm
    Req("confirm rank 1", "POST", "/progression/player-1/ExperienceTrack_Weapon_Axe/1/confirm/public", GS, undefined),
    Req("confirm rank 1 again", "POST", "/progression/player-1/ExperienceTrack_Weapon_Axe/1/confirm/public", GS, undefined),
    Req("confirm rank 0 after 1 keeps 1", "POST", "/progression/player-1/ExperienceTrack_Weapon_Axe/0/confirm/public", GS, undefined),
    Req("confirm a rank the XP does not reach", "POST", "/progression/player-1/ExperienceTrack_Weapon_Axe/2/confirm/public", GS, undefined),
    Req("confirm a rank out of range", "POST", "/progression/player-1/ExperienceTrack_Weapon_Axe/9/confirm/public", GS, undefined),
    Req("confirm an unknown track", "POST", "/progression/player-1/Nothing/1/confirm/public", GS, undefined),
    Req("confirm a bad rank", "POST", "/progression/player-1/ExperienceTrack_Weapon_Axe/-1/confirm/public", GS, undefined),
    Req("confirm a fractional rank", "POST", "/progression/player-1/ExperienceTrack_Weapon_Axe/1.5/confirm/public", GS, undefined),
    Req("confirm for the sentinel", "POST", "/progression/INVALID/ExperienceTrack_Weapon_Axe/1/confirm/public", GS, undefined),
    Req("confirm by a player", "POST", "/progression/player-1/ExperienceTrack_Weapon_Axe/1/confirm/public", "player", "player-1"),
    Req("confirm rank 0 of a track never granted", "POST", "/progression/player-1/ExperienceTrack_Weapon_Sword/0/confirm/public", GS, undefined),
    Req("confirm a mastery rank one short", "POST", "/progression/player-1/MasteryTrack_PlayerLevel/3/confirm/public", GS, undefined),
    Req("confirm a mastery rank two short", "POST", "/progression/player-1/MasteryTrack_Weapon_Sword/3/confirm/public", GS, undefined),
    Req("confirm a prestige rank", "POST", "/progression/player-1/season43/5/confirm/public", GS, undefined),

    // reforge follow-ups (the reset itself is inventory-reforge's route)
    { name: "open a reforge", op: "reforge-open", user: "player-1", track: "ExperienceTrack_Weapon_Axe", progress: 350, banked: 50, cap: 300 },
    Grant("reforge: prestige +1", "player-1", "PrestigeTrack_Weapon_Axe", 1, "r1"),
    Grant("reforge: prestige +1 again", "player-1", "PrestigeTrack_Weapon_Axe", 1, "r2"),
    Grant("reforge: prestige replay", "player-1", "PrestigeTrack_Weapon_Axe", 1, "r1"),
    Grant("reforge: restore of the wrong size", "player-1", "ExperienceTrack_Weapon_Axe", 49, "r3"),
    Grant("reforge: banked restore", "player-1", "ExperienceTrack_Weapon_Axe", 50, "r4"),
    Grant("reforge: banked restore again", "player-1", "ExperienceTrack_Weapon_Axe", 50, "r5"),
    Req("progression list after the reforge", "GET", "/progression/player-1", GS, undefined),
    { name: "open a stale reforge", op: "reforge-open", user: "player-1", track: "ExperienceTrack_Weapon_Sword", progress: 330, banked: 30, cap: 300, ageMs: 600000 },
    Req("load repairs the stale reforge", "GET", "/progression/player-1", GS, undefined),
    Req("load repairs it once", "GET", "/progression/player-1", GS, undefined),

    // the objective report and mastery
    Req("report: first completions", "POST", "/progression/player-1", GS, undefined, {
        progress_tracks: [{ progression_id: "MasteryTrack_PlayerLevel", progress: 2 }, { progression_id: "MasteryTrack_Weapon_Sword", progress: 1 }],
        objectives: [
            { objective_id: "MasteryObjective_Weapon_Sword_Generic_Kills1", value: 5, completed_count: 1 },
            { objective_id: "AchievementObjective_Win", value: 1, completed_count: 1 },
            { objective_id: "MasteryObjective_Weapon_Sword_Generic_Kills2", value: 2, completed_count: 0 },
        ],
    }),
    Req("report: the same again", "POST", "/progression/player-1", GS, undefined, {
        progress_tracks: [{ progression_id: "MasteryTrack_PlayerLevel", progress: 2 }],
        objectives: [{ objective_id: "MasteryObjective_Weapon_Sword_Generic_Kills1", value: 3, completed_count: 1 }],
    }),
    Req("report: credit capped by the claim", "POST", "/progression/player-1", GS, undefined, {
        progress_tracks: [{ progression_id: "MasteryTrack_PlayerLevel", progress: 0 }],
        objectives: [{ objective_id: "MasteryObjective_Behemoth_Embermane_Boop", value: 1, completed_count: 1 }],
    }),
    Req("report: unknown weapon code", "POST", "/progression/player-1", GS, undefined, {
        progress_tracks: [{ progression_id: "MasteryTrack_PlayerLevel", progress: 1 }],
        objectives: [{ objective_id: "MasteryObjective_Weapon_Bow_Kill", value: 1, completed_count: 1 }],
    }),
    Req("report: tracks only", "POST", "/progression/player-1", GS, undefined, { progress_tracks: [{ progression_id: "MasteryTrack_Weapon_Axe", progress: 4 }] }),
    Req("report: empty body", "POST", "/progression/player-1", GS, undefined, {}),
    Req("report: invalid id", "POST", "/progression/player-1", GS, undefined, { objectives: [{ objective_id: "bad id", value: 1 }] }),
    Req("report: negative value", "POST", "/progression/player-1", GS, undefined, { objectives: [{ objective_id: "AchievementObjective_Win", value: -1 }] }),
    Req("report: tracks not an array", "POST", "/progression/player-1", GS, undefined, { progress_tracks: "x" }),
    Req("report: own state by a player token is diagnostics only", "POST", "/progression/player-1", "player", "player-1", { objectives: [{ objective_id: "AchievementObjective_Win", value: 9, completed_count: 1 }] }),
    Req("report: another account by a player token", "POST", "/progression/player-2", "player", "player-1", {}),
    Req("report: the sentinel", "POST", "/progression/INVALID", GS, undefined, { x: 1 }),
    Req("report: the account in the body is ignored for a path user", "POST", "/progression/player-2", GS, undefined, { accountId: "player-1", objectives: [] }),
    Req("objectives after reports", "GET", "/progression/objectives/player-1", "player", "player-1"),
    Req("progression list after mastery", "GET", "/progression/player-1", "player", "player-1"),

    // escalation
    Req("escalation, nothing stored", "GET", "/escalation/ESC_SEASON_6/player-1", "player", "player-1"),
    Req("escalation read of another account", "GET", "/escalation/ESC_SEASON_6/player-2", "player", "player-1"),
    Req("escalation write", "POST", "/escalation/ESC_SEASON_6/player-1", GS, undefined, { escalation_level: 4, next_level_xp: 20, talents_progress: [{ talent_id: "Talent_A", rank: 2 }], unlock_progress: [] }),
    Req("escalation write that lowers the level", "POST", "/escalation/ESC_SEASON_6/player-1", GS, undefined, { escalation_level: 1, next_level_xp: 5 }),
    Req("escalation write of a respec", "POST", "/escalation/ESC_SEASON_6/player-1", GS, undefined, { talents_progress: [] }),
    Req("escalation write with bad values", "POST", "/escalation/ESC_SEASON_6/player-1", GS, undefined, { escalation_level: -1, next_level_xp: 1.5, unlock_progress: "x" }),
    Req("escalation write by the player", "POST", "/escalation/ESC_SEASON_6/player-1", "player", "player-1", { escalation_level: 9 }),
    Req("escalation write by another player", "POST", "/escalation/ESC_SEASON_6/player-2", "player", "player-1", { escalation_level: 9 }),
    Req("escalation read", "GET", "/escalation/ESC_SEASON_6/player-1", GS, undefined),
    Req("escalation, no body", "POST", "/escalation/ESC_SEASON_7/player-1", GS, undefined),

    // encountered content and breadcrumbs
    Req("encountered content add", "POST", "/encountered-content/char-1", "player", "player-1", { content_type: 2, content_id: "BEH_X" }),
    Req("encountered content add again", "POST", "/encountered-content/char-1", "player", "player-1", { content_type: 2, content_id: "BEH_Y" }),
    Req("encountered content add, other category", "POST", "/encountered-content/char-1", "player", "player-1", { content_type: 5, content_id: "ITEM_Z" }),
    Req("encountered content add without a type", "POST", "/encountered-content/char-1", "player", "player-1", { content_id: "ITEM_Z" }),
    Req("encountered content add to a character of another account", "POST", "/encountered-content/char-2", "player", "player-1", { content_type: 2, content_id: "BEH_X" }),
    Req("encountered content get", "GET", "/encountered-content/char-1/2", "player", "player-1"),
    Req("encountered content get, a type with none", "GET", "/encountered-content/char-1/7", "player", "player-1"),
    Req("encountered content get, a type out of range", "GET", "/encountered-content/char-1/9", "player", "player-1"),
    Req("encountered content get, not a number", "GET", "/encountered-content/char-1/x", "player", "player-1"),
    Req("encountered content get, another account's character", "GET", "/encountered-content/char-2/2", "player", "player-1"),
    Req("encountered content get, no user", "GET", "/encountered-content/char-1/2", GS, undefined),
    Req("encountered content query", "POST", "/encountered-content/query/char-1", "player", "player-1", { content_types: [5, 2, 3] }),
    Req("encountered content query, character first", "POST", "/encountered-content/char-1/query", "player", "player-1", { content_types: [2] }),
    Req("encountered content query, no list", "POST", "/encountered-content/char-1/query", "player", "player-1", {}),
    Req("encountered content query, no user", "POST", "/encountered-content/query/char-1", GS, undefined, { content_types: [1, 4] }),
    Req("encountered content query, no character", "POST", "/encountered-content/query", "player", "player-1", { content_types: [1] }),
    Req("encountered content query, no character or list", "POST", "/encountered-content/query", "player", "player-1", {}),
    Req("encountered content, no character id", "POST", "/encountered-content", "player", "player-1", {}),
    Req("encountered content, no character id and a slash", "POST", "/encountered-content/", "player", "player-1", {}),
    Req("breadcrumbs, no character", "GET", "/breadcrumbs/", "player", "player-1"),
    Req("breadcrumbs, created on first read", "GET", "/breadcrumbs/char-1", "player", "player-1"),
    Req("breadcrumbs write", "POST", "/breadcrumbs/char-1", "player", "player-1", { breadcrumbs: [{ id: "a", seen: true }], updateVersion: 3 }),
    Req("breadcrumbs read", "GET", "/breadcrumbs/char-1", "player", "player-1"),
    Req("breadcrumbs of another account's character", "GET", "/breadcrumbs/char-2", "player", "player-1"),
    Req("breadcrumbs write to another account's character", "POST", "/breadcrumbs/char-2", "player", "player-1", { breadcrumbs: [], updateVersion: 1 }),
    Req("breadcrumbs, no user", "GET", "/breadcrumbs/char-1", GS, undefined),
    Req("breadcrumbs write, first for a second character", "POST", "/breadcrumbs/char-3", "player", "player-3", { breadcrumbs: ["x"], updateVersion: 1 }),

    // the Slayer's Path
    Req("pjm, the sentinel", "GET", "/pjm/INVALID", GS, undefined),
    Req("pjm, nothing saved", "GET", "/pjm/player-1", "player", "player-1"),
    Req("pjm, other account", "GET", "/pjm/player-2", "player", "player-1"),
    Req("pjm save by a player", "POST", "/pjm/player-1", "player", "player-1", { nodes: {}, update_version: 2 }),
    Req("pjm save, the sentinel", "POST", "/pjm/INVALID", GS, undefined, { nodes: {}, update_version: 5 }),
    Req("pjm save, the sentinel without a version", "POST", "/pjm/INVALID", GS, undefined, {}),
    Req("pjm first save", "POST", "/pjm/player-1", GS, undefined, { nodes: { A: Node("A", 2), B: Node("B", 0), Gauntlet_Unlock: Node("Gauntlet_Unlock", 2) }, update_version: 2 }),
    Req("pjm read masks Gauntlet_Unlock", "GET", "/pjm/player-1", "player", "player-1"),
    Req("pjm newer save adds nodes", "POST", "/pjm/player-1", GS, undefined, { nodes: { B: Node("B", 2, { objectives: [{ id: "k", progress: 3 }] }), C: Node("C", 0) }, update_version: 3 }),
    Req("pjm stale save keeps stored nodes", "POST", "/pjm/player-1", GS, undefined, { nodes: { A: Node("A", 0), Z: Node("Z", 0) }, update_version: 2 }),
    Req("pjm save with the masked status keeps the unlock", "POST", "/pjm/player-1", GS, undefined, { nodes: { Gauntlet_Unlock: Node("Gauntlet_Unlock", 0) }, update_version: 9 }),
    Req("pjm save without a numeric version", "POST", "/pjm/player-1", GS, undefined, { nodes: { Q: Node("Q", 0) }, update_version: "10" }),
    Req("pjm read", "GET", "/pjm/player-1", GS, undefined),
    Req("pjm without a user", "GET", "/pjm", GS, undefined),
    Req("pjm bootstrap for an account with no save", "GET", "/pjm", "player", "player-2"),
    Req("pjm of the account that saved", "GET", "/pjm", "player", "player-1"),

    // host writes through the hooks
    { name: "host: allow", op: "guard", denial: null, policyRefuses: true, probeRefuses: [] },
    Grant("host grant", "player-1", "ExperienceTrack_Weapon_Sword", 98, "h1", "host", { user: "host-1", roster: "host-1,player-1" }),
    Grant("host grant over the ceiling", "player-1", "ExperienceTrack_Weapon_Axe", 1001, "h2", "host", { user: "host-1" }),
    Grant("host grant to a mastery track", "player-1", "MasteryTrack_PlayerLevel", 1, "h3", "host", { user: "host-1" }),
    Grant("host prestige grant that is not a reforge", "player-1", "PrestigeTrack_Weapon_Axe", 1, "h4", "host", { user: "host-1" }),
    Grant("host grant of the hunt pass", "player-1", "season43", 200, "h5", "host", { user: "host-1" }),
    { name: "host: a session that takes the hunt pass as its own kind", op: "guard", denial: null, policyRefuses: true, probeRefuses: ["XP"] },
    Grant("host hunt pass grant as its own kind", "player-1", "season43", 10, "h6", "host", { user: "host-1" }),
    Grant("host weapon grant refused when only the hunt pass is taken", "player-1", "ExperienceTrack_Weapon_Axe", 10, "h7", "host", { user: "host-1" }),
    { name: "host: observe mode", op: "guard", denial: null, policyRefuses: false, probeRefuses: [] },
    Grant("host grant over the ceiling only logs", "player-1", "ExperienceTrack_Weapon_Axe", 1001, "h8", "host", { user: "host-1" }),
    Grant("host fills the player level track", "player-1", "ExperienceTrack_PlayerLevel", 1000, "h9", "host", { user: "host-1" }),
    Grant("host grant to a full player level track only logs", "player-1", "ExperienceTrack_PlayerLevel", 10, "h10", "host", { user: "host-1" }),
    { name: "host: enforce mode", op: "guard", denial: null, policyRefuses: true, probeRefuses: [] },
    Grant("host grant to a full player level track", "player-1", "ExperienceTrack_PlayerLevel", 10, "h11", "host", { user: "host-1" }),
    Req("host confirm", "POST", "/progression/player-1/ExperienceTrack_Weapon_Axe/2/confirm/public", "host", "host-1"),
    Req("host confirm that raises nothing", "POST", "/progression/player-1/ExperienceTrack_Weapon_Axe/1/confirm/public", "host", "host-1"),
    Req("host report", "POST", "/progression/player-1", "host", "host-1", {
        progress_tracks: [{ progression_id: "MasteryTrack_PlayerLevel", progress: 5 }],
        objectives: [
            { objective_id: "MasteryObjective_Weapon_Sword_Generic_Kills2", value: 500000000, completed_count: 99 },
            { objective_id: "Unknown_Thing", value: 1, completed_count: 1 },
        ],
    }),
    Req("host escalation write", "POST", "/escalation/ESC_SEASON_6/player-1", "host", "host-1", { escalation_level: 6, next_level_xp: 10, talents_progress: [{ talent_id: "Talent_B", rank: 1 }], unlock_progress: [{ reward_id: "Reward_A", collected: true }] }, {}, { roster: "host-1,player-1" }),
    Req("host escalation write of an unknown season", "POST", "/escalation/ESC_SEASON_9/player-1", "host", "host-1", { escalation_level: 6 }),
    Req("host escalation write that is impossible", "POST", "/escalation/ESC_SEASON_6/player-1", "host", "host-1", { escalation_level: 6, next_level_xp: 700, talents_progress: [{ talent_id: "Talent_Z", rank: 1 }] }),
    Req("host escalation read outside its roster", "GET", "/escalation/ESC_SEASON_6/player-1", "host", "host-1", undefined, {}, { roster: "host-1" }),
    Req("host escalation read of its roster", "GET", "/escalation/ESC_SEASON_6/player-1", "host", "host-1", undefined, {}, { roster: "host-1,player-1" }),
    Req("host journey save", "POST", "/pjm/player-1", "host", "host-1", { nodes: { Slayer_11: Node("Slayer_11", 2), D: Node("D", 2), X: Node("X", 2), Ghost: Node("Ghost", 2) }, update_version: 20 }),
    Req("host journey save that locks again", "POST", "/pjm/player-1", "host", "host-1", { nodes: { X: Node("X", 0) }, update_version: 21 }),
    { name: "host: refused", op: "guard", denial: DENIAL, policyRefuses: true, probeRefuses: [] },
    Grant("host grant refused", "player-1", "ExperienceTrack_Weapon_Axe", 5, "h12", "host", { user: "host-1" }),
    Req("host report refused", "POST", "/progression/player-1", "host", "host-1", {}),
    Req("host journey save refused", "POST", "/pjm/player-1", "host", "host-1", { nodes: {} }),
    Req("host escalation write refused", "POST", "/escalation/ESC_SEASON_6/player-1", "host", "host-1", { escalation_level: 7 }),
    Req("host confirm refused", "POST", "/progression/player-1/ExperienceTrack_Weapon_Axe/1/confirm/public", "host", "host-1"),
    Req("host load refused the parity check", "GET", "/progression/player-1", "host", "host-1", undefined, {}, { roster: "host-1,player-1" }),
];

// ---- running them ---------------------------------------------------------------------------------------------------------------------------

let Listening: Server;
let Port = 0;

function Send(step: Step): Promise<{ status: number; body: string }> {
    return new Promise((resolve, reject) => {
        const Headers: Record<string, string> = { ...(step.headers ?? {}) };
        if (step.role !== undefined) Headers["x-test-role"] = step.role;
        if (step.user !== undefined) Headers["x-test-user"] = step.user;
        if (step.roster !== undefined) Headers["x-test-roster"] = step.roster;
        const Payload = step.body === undefined ? undefined : JSON.stringify(step.body);
        if (Payload !== undefined) Headers["content-type"] = "application/json";
        const Request = http.request({ hostname: "127.0.0.1", port: Port, path: step.path, method: step.method, headers: Headers }, (res) => {
            let Text = "";
            res.setEncoding("utf8");
            res.on("data", (Chunk) => { Text += Chunk; });
            res.on("end", () => resolve({ status: res.statusCode ?? 0, body: Text }));
        });
        Request.on("error", reject);
        if (Payload !== undefined) Request.write(Payload);
        Request.end();
    });
}

async function main() {
    const { systemRouter } = require("./routes/system");
    const { progressionRouter } = require("./routes/progression");
    const App = express();
    App.use(express.json());
    App.use(systemRouter);
    App.use(progressionRouter);
    Listening = App.listen(0, "127.0.0.1");
    await new Promise<void>((resolve, reject) => { Listening.once("listening", () => resolve()); Listening.once("error", reject); });
    Port = (Listening.address() as AddressInfo).port;

    for (const User of ["player-1", "player-2", "player-3", "host-1"]) Users.add(User);
    Characters.set("char-1", "player-1");
    Characters.set("char-2", "player-2");
    Characters.set("char-3", "player-3");

    const Recorded: any[] = [];
    for (const Step of Steps) {
        if (Step.op === "guard") {
            Hooks.denial = Step.denial === null ? undefined : Step.denial;
            Hooks.policyRefuses = Step.policyRefuses;
            Hooks.probeRefuses = Step.probeRefuses;
            Recorded.push({ name: Step.name, op: "guard", denial: Step.denial, policyRefuses: Step.policyRefuses, probeRefuses: Step.probeRefuses });
            continue;
        }
        if (Step.op === "reforge-open") {
            const Key = `${Step.user}::${Step.track}`;
            const OpenedAt = new Date(Date.now() - (Step.ageMs ?? 0)).toISOString();
            const Base = Tracks.get(Key) ?? { userId: Step.user, progressionId: Step.track, confirmedFremiumRank: 0, confirmedPremiumRank: 0, createdAt: OpenedAt, updateVersion: 1 };
            Tracks.set(Key, { ...Base, progress: 0, confirmedFremiumRank: 0, confirmedPremiumRank: 0, updatedAt: OpenedAt,
                reforge: { reforgeId: `rf-${Step.track}`, status: "open", priorProgress: Step.progress, cap: Step.cap, banked: Step.banked, bankedRestored: 0, prestigeGranted: false, openedAt: OpenedAt } });
            Recorded.push({ name: Step.name, op: "reforge-open", user: Step.user, track: Step.track, progress: Step.progress, banked: Step.banked, cap: Step.cap, ageMs: Step.ageMs ?? 0 });
            continue;
        }
        Hooks.authorized.length = 0;
        Hooks.records.length = 0;
        Hooks.notes.length = 0;
        const Answer = await Send(Step);
        Recorded.push({
            name: Step.name,
            request: { method: Step.method, path: Step.path, role: Step.role, user: Step.user, roster: Step.roster, headers: Step.headers ?? {}, body: Step.body },
            response: { status: Answer.status, body: Answer.body },
            hooks: { authorized: [...Hooks.authorized], records: Hooks.records.map((R) => ({ ...R })), notes: Hooks.notes.length },
        });
    }
    await new Promise<void>((resolve) => Listening.close(() => resolve()));
    const Out = process.env.GOLDEN_OUT ?? path.join(Fixture, "scenarios-1.14.7.json");
    fs.writeFileSync(Out, JSON.stringify({
        version: "1.14.7",
        users: ["player-1", "player-2", "player-3", "host-1"],
        characters: [["player-1", "char-1"], ["player-2", "char-2"], ["player-3", "char-3"]],
        steps: Recorded,
    }, null, 2) + "\n");
    console.log(`recorded ${Recorded.length} steps to ${Out}`);
    process.exit(0);
}

main().catch((error) => { console.error(error); process.exit(1); });
