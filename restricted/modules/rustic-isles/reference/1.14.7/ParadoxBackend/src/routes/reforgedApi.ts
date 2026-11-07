/*
 * Copyright (C) 2026 Dainty Lass contributors
 * SPDX-License-Identifier: AGPL-3.0-only
 *
 * New to this backend: nothing in this file comes from Mystic Paradox.
 */

// The launcher API: how the Dainty Lass launcher, the content server, the admin service and the host kit talk to
// this backend. Modelled on Dauntless Revived's /undaunted/api: invite codes, one account key per player and a
// live server status. The paths and names are ones installs depend on (docs/dainty-lass-plan.md, DD1).
//
//   POST   /reforged/api/Register             {Username, InviteCode?}   -> 200 {UUK}
//   GET    /reforged/api/GetUserInfo          x-reforged-user-api-key   -> {UserId, Username, IsAdmin}
//   GET    /reforged/api/ServerStatus         the full view with an accepted key, the limited one without
//   GET    /reforged/api/RegistrationStatus                             -> {mode}
//   GET    /dauntless-status                  the probe older launchers send to tell a server from anything else
//
// For the host kit only (this machine, never through the gateway, x-reforged-admin-key):
//   POST   /reforged/api/CreateInvite         {uses?, infinite?, name?} -> {code}
//   GET    /reforged/api/InviteCodes                                    -> {InviteCodes: [...]}
//   DELETE /reforged/api/InviteCode/:code
//   POST   /reforged/api/SetAccountStatus     {Username, Status: active|banned|disabled, Reason?}
//   POST   /reforged/api/Kick                 {Username}                -> {UserId, Username, Closed}
//
// The key is the game's login: with AUTH_MODE=APIKEY the client trades it for a session token at
// /account/api/oauth/token (routes/eos.ts). Only its HMAC is stored (security/apiKeyHash.ts). An account made here
// is a launcher account like any other, so eligibility, bans and name lookups need nothing of their own.
//
// The gateway's tests read this file: every route is registered as `reforgedApiRouter.<method>("<path>", ...)`.

import crypto from "node:crypto";
import path from "node:path";
import { Router, type NextFunction, type Request, type Response } from "express";
import { MongoServerError } from "mongodb";
import { logger } from "../logger";
import { GetRepositories, type AccountOperationalStatus, type LauncherAccountRecord } from "../persistence";
import { GetMongoClient, GetMongoDb } from "../persistence/mongo/client";
import { Collections } from "../persistence/mongo/collections";
import { GetUserIDForAPIKey } from "../controllers/auth";
import { HashApiKey } from "../security/apiKeyHash";
import { IsAccountEligible } from "../security/accountEligibility";
import { sessionRegistry } from "../realtime/SessionRegistry";
import {
    type GameDataKind,
    type PlayerWhere,
    type RegistrationMode,
    ClassifyGameData,
    DisconnectAccount,
    InviteNoteFrom,
    IsInviteCode,
    IsLocalAdminCall,
    LooksLikeAccountKey,
    MakeAccountKey,
    MakeInviteCode,
    MAX_INVITE_USES,
    NormalizeUsername,
    Refusal,
    RegistrationModeFrom,
    StatusInstancesFrom,
    USER_KEY_HEADER
} from "../launcherApiRules";

export const reforgedApiRouter = Router();

// The collection existing servers already hold their invites in.
const INVITES = "reforgedInvites";
const STATUS_CACHE_MS = 5_000;
const GAME_DATA_CACHE_MS = 60_000;
const DIRECTOR_TIMEOUT_MS = 2_000;
const MAX_PLAYERS_LISTED = 500;
const MAX_INVITES_LISTED = 500;
const BOOTED_AT = Date.now();

type InviteRecord = {
    _id: string;
    usesRemaining: number;
    infiniteUses: boolean;
    note?: string;
    createdAt: string;
};

async function InviteCollection() {
    return (await GetMongoDb()).collection<InviteRecord>(INVITES);
}

function Body(req: Request): Record<string, unknown> {
    return req.body !== null && typeof req.body === "object" ? (req.body as Record<string, unknown>) : {};
}

function Mode(): RegistrationMode {
    return RegistrationModeFrom(process.env.REGISTRATION_MODE);
}

function Problem(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}

// ------------------------------------------------------------------ who holds a key

type KeyHolder = {
    userId: string;
    username: string;
    isAdmin: boolean;
    account: LauncherAccountRecord | undefined;
};

async function HolderOf(key: unknown): Promise<KeyHolder | undefined> {
    if (!LooksLikeAccountKey(key)) return undefined;
    const UserId = await GetUserIDForAPIKey(key);
    if (UserId === undefined) return undefined;
    const Repos = GetRepositories();
    const Account = await Repos.launcherAccounts.findByUserId(UserId);
    const Username = Account?.displayName ?? (await Repos.accounts.findByUserId(UserId))?.name ?? UserId;
    return { userId: UserId, username: Username, isAdmin: Account?.roles.includes("admin") === true, account: Account };
}

function Eligible(holder: KeyHolder): boolean {
    return holder.account === undefined || IsAccountEligible(holder.account);
}

// ------------------------------------------------------------------ the status

function ServerName(): string {
    const Name = (process.env.REFORGED_SERVER_NAME ?? "").replace(/[\u0000-\u001f\u007f-\u009f]/g, " ").trim();
    return Name === "" ? "Dainty Lass server" : Name.slice(0, 64);
}

function ServerVersion(): string {
    return (process.env.REFORGED_SERVER_VERSION ?? "").trim().slice(0, 48) || "0.1.0";
}

function HttpsUrlOrNull(raw: string | undefined): string | null {
    if (!raw) return null;
    try {
        const Url = new URL(raw.trim());
        return Url.protocol === "https:" && Url.username === "" && Url.password === "" ? Url.toString() : null;
    } catch {
        return null;
    }
}

let GameDataSeen: { at: number; kind: GameDataKind } | undefined;

function GameData(): GameDataKind {
    if (GameDataSeen && Date.now() - GameDataSeen.at < GAME_DATA_CACHE_MS) return GameDataSeen.kind;
    const Dir = process.env.PARADOX_GAME_DATA_DIR
        ? path.resolve(process.env.PARADOX_GAME_DATA_DIR)
        : path.join(__dirname, "..", "..", "game-data");
    GameDataSeen = { at: Date.now(), kind: ClassifyGameData(Dir) };
    return GameDataSeen.kind;
}

async function DirectorInstances(): Promise<unknown> {
    const Director = (process.env.DEPLOYSERVER_URL ?? "").trim();
    if (Director === "") return [];
    const Abort = new AbortController();
    const Timer = setTimeout(() => Abort.abort(), DIRECTOR_TIMEOUT_MS);
    try {
        const Answer = await fetch(`http://${Director}/api/instances`, { signal: Abort.signal });
        return Answer.ok ? await Answer.json() : [];
    } catch {
        return [];
    } finally {
        clearTimeout(Timer);
    }
}

type Player = { name: string; where: PlayerWhere; instance: string | null };

async function PlayersOnline(huntOf: Map<string, string>): Promise<Player[]> {
    const Repos = GetRepositories();
    const Players: Player[] = [];
    for (const UserId of sessionRegistry.onlineAccountIds().slice(0, MAX_PLAYERS_LISTED)) {
        const Account = await Repos.launcherAccounts.findByUserId(UserId).catch(() => undefined);
        const Name = Account?.displayName ?? (await Repos.accounts.findByUserId(UserId).catch(() => undefined))?.name;
        if (!Name) continue;
        const Hunt = huntOf.get(UserId) ?? null;
        Players.push({ name: Name.slice(0, 32), where: Hunt === null ? "unknown" : "hunt", instance: Hunt });
    }
    Players.sort((A, B) => A.name.localeCompare(B.name, undefined, { sensitivity: "base" }));
    return Players;
}

type View = "full" | "limited";
const StatusSeen = new Map<View, { at: number; body: unknown }>();

function ForgetStatus(): void {
    StatusSeen.clear();
}

/** The status body (crates/dainty-protocol status.rs, field for field). The limited view hides who is online. */
export async function ServerStatusBody(view: View): Promise<unknown> {
    const Seen = StatusSeen.get(view);
    if (Seen && Date.now() - Seen.at < STATUS_CACHE_MS) return Seen.body;

    const Changelist = Number(process.env.TARGET_CHANGELIST ?? "647472") || 647472;
    const ExecutableHash = (process.env.APPROVED_EXECUTABLE_SHA256 ?? "").toLowerCase();
    const Common = {
        name: ServerName(),
        online: true,
        version: ServerVersion(),
        commit: (process.env.REFORGED_SOURCE_COMMIT ?? "").trim().slice(0, 64),
        sourceUrl: HttpsUrlOrNull(process.env.REFORGED_SOURCE_URL),
        registration: Mode(),
        uptimeSeconds: Math.floor((Date.now() - BOOTED_AT) / 1000),
        contentPort: null,
        game: {
            build: "rel-1.14.7-Archon",
            changelist: Changelist,
            executableSha256: /^[0-9a-f]{64}$/.test(ExecutableHash) ? ExecutableHash : null
        },
        gameData: GameData(),
        chat: process.env.REALTIME_XMPP_ENABLED === "true"
    };

    let BodyOut: unknown;
    if (view === "limited") {
        BodyOut = { ...Common, playersOnline: 0, players: [], instances: [], limited: true };
    } else {
        const { instances, huntOf } = StatusInstancesFrom(await DirectorInstances());
        const Players = await PlayersOnline(huntOf);
        BodyOut = { ...Common, playersOnline: Players.length, players: Players, instances, limited: false };
    }
    StatusSeen.set(view, { at: Date.now(), body: BodyOut });
    return BodyOut;
}

// ------------------------------------------------------------------ players

class Refused extends Error {
    constructor(readonly code: "invite_invalid" | "username_taken") {
        super(code);
        this.name = "Refused";
    }
}

reforgedApiRouter.post("/reforged/api/Register", async (req, res) => {
    const RegistrationMode = Mode();
    if (RegistrationMode === "NONE") {
        res.status(400).json(Refusal("registration_closed"));
        return;
    }
    const Fields = Body(req);
    if (typeof Fields.Username !== "string" || (Fields.InviteCode !== undefined && typeof Fields.InviteCode !== "string")) {
        res.status(400).json(Refusal("bad_request"));
        return;
    }
    const Username = NormalizeUsername(Fields.Username);
    if (Username === undefined) {
        res.status(400).json(Refusal("username_invalid"));
        return;
    }
    const Code = typeof Fields.InviteCode === "string" ? Fields.InviteCode.trim() : "";
    if (RegistrationMode === "INVITECODE" && !IsInviteCode(Code)) {
        res.status(401).json(Refusal("invite_invalid"));
        return;
    }

    const UserId = crypto.randomUUID();
    const Key = MakeAccountKey();
    const Account: LauncherAccountRecord = {
        userId: UserId,
        displayName: Username,
        displayNameNormalized: Username.toLowerCase(),
        status: "active",
        approvalStatus: "approved",
        roles: ["player"],
        createdAt: new Date().toISOString(),
        usernameSet: true
    };

    // One transaction for the code, the account and the key: a taken name costs no invite use, and no account is
    // ever written without its key. A refusal thrown inside rolls everything back.
    const Session = (await GetMongoClient()).startSession();
    try {
        await Session.withTransaction(async () => {
            const Db = await GetMongoDb();
            if (RegistrationMode === "INVITECODE") {
                const Spent = await Db.collection<InviteRecord>(INVITES).findOneAndUpdate(
                    { _id: Code, $or: [{ infiniteUses: true }, { usesRemaining: { $gt: 0 } }] },
                    [{ $set: { usesRemaining: { $cond: ["$infiniteUses", "$usesRemaining", { $subtract: ["$usesRemaining", 1] }] } } }],
                    { session: Session }
                );
                if (Spent === null || Spent === undefined) throw new Refused("invite_invalid");
            }
            try {
                await GetRepositories().launcherAccounts.create(Account, Session);
            } catch (error) {
                if (error instanceof MongoServerError && error.code === 11000) throw new Refused("username_taken");
                throw error;
            }
            await Db.collection(Collections.UserApiKeys).insertOne(
                { _id: UserId as never, userId: UserId, keyHash: HashApiKey(Key, "user") },
                { session: Session }
            );
        });
    } catch (error) {
        if (error instanceof Refused) {
            res.status(error.code === "invite_invalid" ? 401 : 409).json(Refusal(error.code));
            return;
        }
        logger.error(`[launcher-api] registration failed: ${Problem(error)}`);
        res.status(500).send();
        return;
    } finally {
        await Session.endSession();
    }

    logger.info(`[launcher-api] ${Username} registered as ${UserId}`);
    ForgetStatus();
    res.status(200).json({ UUK: Key });
});

reforgedApiRouter.get("/reforged/api/GetUserInfo", async (req, res) => {
    const Holder = await HolderOf(req.headers[USER_KEY_HEADER]).catch(() => undefined);
    if (Holder === undefined) {
        res.status(401).json({ error: "unauthorized" });
        return;
    }
    if (!Eligible(Holder)) {
        const Account = Holder.account as LauncherAccountRecord;
        res.status(403).json({
            error: Account.status === "banned" ? "account_banned" : "account_disabled",
            reason: (Account.approvalReason ?? "").slice(0, 500) || "Ask the server's host."
        });
        return;
    }
    res.status(200).json({ UserId: Holder.userId, Username: Holder.username, IsAdmin: Holder.isAdmin });
});

reforgedApiRouter.get("/reforged/api/ServerStatus", async (req, res) => {
    const Holder = await HolderOf(req.headers[USER_KEY_HEADER]).catch(() => undefined);
    const View: View = Holder !== undefined && Eligible(Holder) ? "full" : "limited";
    try {
        const Answer = await ServerStatusBody(View);
        res.status(200);
        res.set("Cache-Control", "no-store");
        res.vary(USER_KEY_HEADER);
        res.json(Answer);
    } catch (error) {
        logger.error(`[launcher-api] status failed: ${Problem(error)}`);
        res.status(500).send();
    }
});

reforgedApiRouter.get("/reforged/api/RegistrationStatus", (_req, res) => {
    res.status(200).json({ mode: Mode() });
});

reforgedApiRouter.get("/dauntless-status", (_req, res) => {
    res.status(200).json({
        "show-status": true,
        name: ServerName(),
        version: ServerVersion(),
        sourceUrl: HttpsUrlOrNull(process.env.REFORGED_SOURCE_URL)
    });
});

// ------------------------------------------------------------------ the host

function HostOnly(req: Request, res: Response, next: NextFunction): void {
    if (!IsLocalAdminCall(req, process.env.REFORGED_ADMIN_KEY)) {
        res.status(403).json({ error: "forbidden" });
        return;
    }
    next();
}

async function AccountNamed(raw: unknown): Promise<LauncherAccountRecord | undefined | null> {
    const Username = NormalizeUsername(raw);
    if (Username === undefined) return null;
    return GetRepositories().launcherAccounts.findByDisplayNameNormalized(Username.toLowerCase());
}

reforgedApiRouter.post("/reforged/api/CreateInvite", HostOnly, async (req, res) => {
    const Fields = Body(req);
    const Uses = Fields.uses ?? 1;
    const Infinite = Fields.infinite === true;
    const UsesOk = Number.isSafeInteger(Uses) && (Uses as number) >= 1 && (Uses as number) <= MAX_INVITE_USES;
    if (!UsesOk || (Fields.name !== undefined && typeof Fields.name !== "string")) {
        res.status(400).json({ error: "bad_request", message: `uses is a whole number from 1 to ${MAX_INVITE_USES}; name is text` });
        return;
    }
    const Note = InviteNoteFrom(Fields.name);
    const Invites = await InviteCollection();
    // A code is 60 random bits: a clash is a curiosity, but it is retried rather than reported.
    for (let Attempt = 0; Attempt < 5; Attempt++) {
        const Code = MakeInviteCode();
        try {
            await Invites.insertOne({ _id: Code, usesRemaining: Uses as number, infiniteUses: Infinite, note: Note, createdAt: new Date().toISOString() });
        } catch (error) {
            if (error instanceof MongoServerError && error.code === 11000) continue;
            throw error;
        }
        logger.info(`[launcher-api] invite made: ${Infinite ? "unlimited" : `${Uses} use(s)`}${Note ? ` for ${Note}` : ""}`);
        res.status(200).json({ code: Code });
        return;
    }
    res.status(500).send();
});

reforgedApiRouter.get("/reforged/api/InviteCodes", HostOnly, async (_req, res) => {
    const Invites = await InviteCollection();
    const Records = await Invites.find({}).sort({ createdAt: -1 }).limit(MAX_INVITES_LISTED).toArray();
    res.status(200).json({
        InviteCodes: Records.map((Record) => ({
            code: Record._id,
            usesRemaining: Record.usesRemaining,
            infiniteUses: Record.infiniteUses,
            note: Record.note ?? null,
            createdAt: Record.createdAt
        }))
    });
});

reforgedApiRouter.delete("/reforged/api/InviteCode/:code", HostOnly, async (req, res) => {
    const Code = String(req.params.code ?? "");
    if (!IsInviteCode(Code)) {
        res.status(400).json({ error: "bad_request" });
        return;
    }
    const Gone = await (await InviteCollection()).deleteOne({ _id: Code });
    res.status(Gone.deletedCount === 1 ? 200 : 404).send();
});

const ACCOUNT_STATUSES: readonly AccountOperationalStatus[] = ["active", "banned", "disabled"];

reforgedApiRouter.post("/reforged/api/SetAccountStatus", HostOnly, async (req, res) => {
    const Fields = Body(req);
    const Status = ACCOUNT_STATUSES.find((Known) => Known === Fields.Status);
    const Account = Status === undefined ? null : await AccountNamed(Fields.Username);
    if (Account === null) {
        res.status(400).json({ error: "bad_request", message: "Username and Status (active, banned or disabled) are required." });
        return;
    }
    if (Account === undefined) {
        res.status(404).json(Refusal("not_found"));
        return;
    }
    const Changed = await GetRepositories().launcherAccounts.setAccessState(Account.userId, {
        status: Status,
        approvalUpdatedAt: new Date().toISOString(),
        approvalUpdatedBy: "host",
        approvalReason: typeof Fields.Reason === "string" ? Fields.Reason.slice(0, 500) : undefined
    });
    ForgetStatus();
    logger.info(`[launcher-api] ${Account.displayName} (${Account.userId}) set ${Status}`);
    res.status(200).json({ UserId: Account.userId, Username: Account.displayName, Status: Changed?.status ?? Status });
});

// The admin service's kick (rust/services/admin, MIGRATION_PLAN.md RD25; docs/security/admin-api.md). The player is
// dropped, not banned, and logs in again with their key.
reforgedApiRouter.post("/reforged/api/Kick", HostOnly, async (req, res) => {
    const Account = await AccountNamed(Body(req).Username);
    if (Account === null) {
        res.status(400).json({ error: "bad_request", message: "Username is required." });
        return;
    }
    if (Account === undefined) {
        res.status(404).json(Refusal("not_found"));
        return;
    }
    const Repos = GetRepositories();
    const Closed = await DisconnectAccount(Account.userId, {
        connectionsFor: (Id) => sessionRegistry.connectionsFor(Id),
        revokeRefreshSessions: (Id) => Repos.refreshSessions.revokeAllForUser(Id),
        revokeExchangeCodes: (Id) => Repos.gameExchangeCodes.revokeUnusedForUser(Id)
    });
    ForgetStatus();
    logger.info(`[launcher-api] ${Account.displayName} (${Account.userId}) kicked, ${Closed} chat connection(s) closed`);
    res.status(200).json({ UserId: Account.userId, Username: Account.displayName, Closed });
});
