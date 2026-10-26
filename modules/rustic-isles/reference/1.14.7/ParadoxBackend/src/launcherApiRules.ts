/*
 * Copyright (C) 2026 DaintyLass contributors
 * SPDX-License-Identifier: AGPL-3.0-only
 *
 * New to this backend: nothing in this file comes from Mystic Paradox.
 */

// The rules of the launcher API (routes/reforgedApi.ts), kept away from the database and the environment so
// they are tested on their own. The contract they implement is the Rust side's: crates/protocol (account.rs,
// status.rs), the launcher's net/hostapi, the content server's auth and the admin service's upstream.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import type { Request } from "express";

export const USER_KEY_HEADER = "x-reforged-user-api-key";
export const ADMIN_KEY_HEADER = "x-reforged-admin-key";
// The gateway stamps every request it forwards. Whatever the socket says, a request carrying it came from outside.
export const GATEWAY_HEADER = "x-reforged-gateway";

// ------------------------------------------------------------------ registration mode

export type RegistrationMode = "OPEN" | "INVITECODE" | "NONE";

/** REGISTRATION_MODE: OPEN or NONE when it says so; invite codes otherwise, unset included. */
export function RegistrationModeFrom(raw: string | undefined): RegistrationMode {
    switch ((raw ?? "").trim().toUpperCase()) {
        case "OPEN": return "OPEN";
        case "NONE": return "NONE";
        default: return "INVITECODE";
    }
}

// ------------------------------------------------------------------ names, codes, keys

export const USERNAME_MIN = 3;
export const USERNAME_MAX = 16;
const USERNAME = /^[A-Za-z0-9_]+$/;

/** A username as the server keeps it: trimmed, 3-16 letters, digits and underscores. Uniqueness ignores case. */
export function NormalizeUsername(raw: unknown): string | undefined {
    if (typeof raw !== "string") return undefined;
    const Name = raw.trim();
    if (Name.length < USERNAME_MIN || Name.length > USERNAME_MAX || !USERNAME.test(Name)) return undefined;
    return Name;
}

// Letters and digits that are not mistaken for each other when read aloud or copied by hand: no 0 and O, no 1, I
// and L. Three groups of four give about 60 random bits, which nobody guesses through the gateway's rate limit.
const CODE_ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const CODE_GROUPS = 3;
const CODE_GROUP_LENGTH = 4;

export function MakeInviteCode(pick: (bound: number) => number = (bound) => crypto.randomInt(bound)): string {
    const Groups: string[] = [];
    for (let Group = 0; Group < CODE_GROUPS; Group++) {
        let Text = "";
        for (let Char = 0; Char < CODE_GROUP_LENGTH; Char++) Text += CODE_ALPHABET[pick(CODE_ALPHABET.length)];
        Groups.push(Text);
    }
    return Groups.join("-");
}

/** Anything a host could have typed as a code, not only the shape MakeInviteCode produces. */
export function IsInviteCode(raw: unknown): raw is string {
    return typeof raw === "string" && /^[A-Za-z0-9-]{4,64}$/.test(raw);
}

export const MAX_INVITE_USES = 1000;

/** The note on an invite ("for Alex"): control characters become spaces, then trimmed and capped at 64. */
export function InviteNoteFrom(raw: unknown): string | undefined {
    if (typeof raw !== "string") return undefined;
    const Note = raw.replace(/[\u0000-\u001f\u007f-\u009f]/g, " ").trim().slice(0, 64);
    return Note === "" ? undefined : Note;
}

/** Account keys: the RRK_ prefix and 48 hex digits (192 random bits). Only plain characters, since the key is
 *  passed on the game's command line as -AUTH_PASSWORD. */
export const ACCOUNT_KEY_PREFIX = "RRK_";

export function MakeAccountKey(): string {
    return ACCOUNT_KEY_PREFIX + crypto.randomBytes(24).toString("hex");
}

/** Worth looking up at all: a key-shaped string, this server's or another's. */
export function LooksLikeAccountKey(raw: unknown): raw is string {
    return typeof raw === "string" && /^[A-Za-z0-9_-]{8,128}$/.test(raw);
}

// ------------------------------------------------------------------ refusals

export type RefusalCode =
    | "bad_request"
    | "invite_invalid"
    | "not_found"
    | "registration_closed"
    | "username_invalid"
    | "username_taken";

const REFUSAL_TEXT: Record<RefusalCode, string> = {
    bad_request: "The body is not JSON, or a field is missing or has the wrong type.",
    invite_invalid: "That invite code is not valid, or has been used up.",
    not_found: "No account has that name.",
    registration_closed: "This server is not taking registrations.",
    username_invalid: `A username is ${USERNAME_MIN} to ${USERNAME_MAX} characters: letters, digits and underscore.`,
    username_taken: "That name is taken."
};

/** `{error, message}`: the code the launcher maps to its own text, and a sentence for anyone else. */
export function Refusal(code: RefusalCode): { error: RefusalCode; message: string } {
    return { error: code, message: REFUSAL_TEXT[code] };
}

// ------------------------------------------------------------------ the host's own calls

function IsLoopback(address: string | undefined): boolean {
    if (!address) return false;
    return address === "::1" || /^(::ffff:)?127(\.\d{1,3}){3}$/.test(address);
}

function Digest(text: string): Buffer {
    return crypto.createHash("sha256").update(text, "utf8").digest();
}

/** An admin route is for the host kit on this machine: a loopback socket, nothing forwarded (no gateway stamp,
 *  no X-Forwarded-For) and the configured admin key, compared in constant time. A short or missing key refuses
 *  everything, so a server set up without one has no admin routes rather than open ones. */
export function IsLocalAdminCall(req: Pick<Request, "headers" | "socket">, adminKey: string | undefined): boolean {
    if (!adminKey || adminKey.length < 32) return false;
    if (!IsLoopback(req.socket?.remoteAddress)) return false;
    if (req.headers[GATEWAY_HEADER] !== undefined || req.headers["x-forwarded-for"] !== undefined) return false;
    const Given = req.headers[ADMIN_KEY_HEADER];
    if (typeof Given !== "string") return false;
    return crypto.timingSafeEqual(Digest(Given), Digest(adminKey));
}

// ------------------------------------------------------------------ the status

export type PlayerWhere = "menu" | "city" | "hunt" | "dojo" | "tutorial" | "unknown";
export type InstanceKind = "city" | "hunt" | "dojo" | "tutorial";

export type StatusInstance = {
    id: string;
    kind: InstanceKind;
    title: string;
    map: string;
    behemoth: string | null;
    players: number;
    maxPlayers: number;
    startedAt: string | null;
};

const INSTANCE_KINDS: readonly InstanceKind[] = ["city", "hunt", "dojo", "tutorial"];
const MAX_INSTANCES = 100;
const MAX_PLAYERS_PER_HUNT = 16;

function Text(value: unknown, max: number): string | undefined {
    return typeof value === "string" ? value.slice(0, max) : undefined;
}

function Count(value: unknown): number {
    return typeof value === "number" && Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

function IsoDate(value: unknown): string | null {
    if (typeof value !== "string") return null;
    const Time = Date.parse(value);
    return Number.isFinite(Time) ? new Date(Time).toISOString() : null;
}

/** "/Game/Maps/islands/foo/foo_persistent" -> "foo_persistent"; "/Game/Behemoths/Skarn.Skarn_C" -> "Skarn". */
export function AssetLeaf(asset: string): string {
    const Last = asset.slice(asset.lastIndexOf("/") + 1);
    return Last.split(".")[0].slice(0, 64);
}

/** What the launcher lists an instance as. Hunts are named after the behemoth, else the map. */
export function InstanceTitle(kind: InstanceKind, behemoth: string | null, map: string): string {
    switch (kind) {
        case "city": return "Ramsgate";
        case "dojo": return "Training Grounds";
        case "tutorial": return "Tutorial";
        case "hunt": break;
    }
    if (behemoth) return AssetLeaf(behemoth).replace(/^BPC?_/, "").replace(/_C$/, "").replace(/_/g, " ").slice(0, 64);
    const FromMap = AssetLeaf(map).replace(/_persistent$/i, "").replace(/_/g, " ").slice(0, 64);
    return FromMap === "" ? "Hunt" : FromMap;
}

/** The Director's GET /api/instances, read field by field: it runs on this machine, but one odd entry must never
 *  take the status page down. Also which hunt each player is in, for the players list. */
export function StatusInstancesFrom(raw: unknown): { instances: StatusInstance[]; huntOf: Map<string, string> } {
    const Instances: StatusInstance[] = [];
    const HuntOf = new Map<string, string>();
    if (!Array.isArray(raw)) return { instances: Instances, huntOf: HuntOf };
    for (const Entry of raw.slice(0, MAX_INSTANCES)) {
        if (typeof Entry !== "object" || Entry === null) continue;
        const Item = Entry as Record<string, unknown>;
        const Id = Text(Item.id, 64);
        const Kind = INSTANCE_KINDS.find((Known) => Known === Item.kind);
        if (!Id || !Kind) continue;
        const Map = Text(Item.map, 160) ?? "";
        const Behemoth = Text(Item.behemoth, 160) ?? null;
        Instances.push({
            id: Id,
            kind: Kind,
            title: InstanceTitle(Kind, Behemoth, Map),
            map: AssetLeaf(Map),
            behemoth: Behemoth === null ? null : AssetLeaf(Behemoth),
            players: Count(Item.players),
            maxPlayers: Count(Item.maxPlayers),
            startedAt: IsoDate(Item.startedAt)
        });
        if (Kind === "hunt" && Array.isArray(Item.playerIds)) {
            for (const PlayerId of Item.playerIds.slice(0, MAX_PLAYERS_PER_HUNT)) {
                if (typeof PlayerId === "string" && PlayerId.length <= 128) HuntOf.set(PlayerId, Id);
            }
        }
    }
    return { instances: Instances, huntOf: HuntOf };
}

// ------------------------------------------------------------------ game data

/** "real": every required file is there and none is the shipped example; "synthetic": the examples are in use
 *  (the services start, nobody can play); "missing": a file is absent. The launcher tells players before PLAY. */
export type GameDataKind = "real" | "synthetic" | "missing";

export const GAME_DATA_FILES: readonly string[] = [
    "progression_config.json",
    "slayers_path.json",
    "slayers_path_definitions.json",
    "ladyluck_store.json",
    "platinum_store.json",
    "reward_cache_store.json",
    "inventory_storage_policy.json"
];

function SameBytes(a: string, b: string): boolean {
    try {
        return fs.readFileSync(a).equals(fs.readFileSync(b));
    } catch {
        return false;
    }
}

export function ClassifyGameData(dir: string, files: readonly string[] = GAME_DATA_FILES): GameDataKind {
    let AnyExample = false;
    for (const Name of files) {
        const File = path.join(dir, Name);
        if (!fs.existsSync(File)) return "missing";
        if (SameBytes(File, path.join(dir, Name.replace(/\.json$/, ".example.json")))) AnyExample = true;
    }
    return AnyExample ? "synthetic" : "real";
}
