/**
 * A refusal the person can act on. The code is a key under `errors.*` in messages/en.json, so the
 * sentence is the catalog's, never built here; `detail` fills a {detail} placeholder where one exists.
 */
export const ERROR_CODES = [
  "unauthenticated",
  "forbidden",
  "invalid_input",
  "not_found",
  "conflict",
  "reason_required",
  "self_lockout",
  "last_owner",
  "not_revertible",
  "already_reverted",
  "regex_invalid",
  "game_unavailable",
  "game_refused",
  "game_not_configured",
  "controller_disabled",
  "controller_offline",
  "game_timeout",
  "forbidden_call",
  "protected_account",
  "already_banned",
  "not_banned",
  "not_timed_out",
  "unknown",
] as const;

export type ErrorCode = (typeof ERROR_CODES)[number];

export class DomainError extends Error {
  constructor(
    readonly code: ErrorCode,
    readonly detail?: string,
  ) {
    super(detail ? `${code}: ${detail}` : code);
    this.name = "DomainError";
  }
}

export type ActionResult<T = undefined> =
  | { ok: true; value?: T }
  | { ok: false; code: ErrorCode; detail?: string };

/** Runs a server action body, turning a DomainError into a result and anything else into "unknown". */
export async function attempt<T>(fn: () => Promise<T>): Promise<ActionResult<T>> {
  try {
    return { ok: true, value: await fn() };
  } catch (error) {
    if (error instanceof DomainError) return { ok: false, code: error.code, detail: error.detail };
    // Next's redirect() and notFound() are thrown; they must keep propagating.
    if (error && typeof error === "object" && "digest" in error) throw error;
    console.error("[action]", error);
    return { ok: false, code: "unknown" };
  }
}
