/*
 * Copyright (C) 2026 Dainty Lass contributors
 * SPDX-License-Identifier: AGPL-3.0-only
 */

// The names from before the Rustic Isles rename (2026-11-01), still accepted for one release so existing env files,
// runtimes and launchers keep working. Imported first by server.ts, so the config modules read the new names.

import type { NextFunction, Request, Response } from "express";

export const ENV_PREFIX = "RUSTIC_ISLES_";
export const LEGACY_ENV_PREFIX = "MYSTICPARADOX_";
export const HEADER_PREFIX = "x-rustic-isles-";
export const LEGACY_HEADER_PREFIX = "x-mysticparadox-";
export const RUNTIME_DLL = "Rustic-Isles.dll";
export const LEGACY_RUNTIME_DLL = "MysticParadox.dll";

const Warned = new Set<string>();

/**
 * Gives each MYSTICPARADOX_* variable the new name doesn't set its RUSTIC_ISLES_* name. The old one stays, so a game
 * server running an older runtime still finds it. One warning per name, never with the value.
 */
export function AcceptLegacyEnv(env: NodeJS.ProcessEnv = process.env): void {
    for (const name of Object.keys(env)) {
        if (!name.startsWith(LEGACY_ENV_PREFIX)) continue;
        const renamed = ENV_PREFIX + name.slice(LEGACY_ENV_PREFIX.length);
        if (env[renamed] !== undefined) continue;
        env[renamed] = env[name];
        if (!Warned.has(renamed)) {
            Warned.add(renamed);
            console.warn(`${name} is deprecated and will stop working in the next release: rename it to ${renamed}`);
        }
    }
}

/** Copies each x-mysticparadox-* header the request doesn't also send under its x-rustic-isles-* name. */
export function AcceptLegacyHeaders(req: Request, _res: Response, next: NextFunction): void {
    for (const name of Object.keys(req.headers)) {
        if (!name.startsWith(LEGACY_HEADER_PREFIX)) continue;
        const renamed = HEADER_PREFIX + name.slice(LEGACY_HEADER_PREFIX.length);
        if (req.headers[renamed] === undefined) req.headers[renamed] = req.headers[name];
    }
    next();
}

/** The runtime DLL under its new name when a launcher reports the old one. */
export function CanonicalArtifactName(name: string): string {
    return name.toLowerCase() === LEGACY_RUNTIME_DLL.toLowerCase() ? RUNTIME_DLL : name;
}

AcceptLegacyEnv();
