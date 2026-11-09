import { createHash, createHmac } from "node:crypto";
import { ADMIN_OPS, type AdminOp, backendAllowed } from "./ops";
import type { Operator } from "./types";

/**
 * The calls themselves, made from wherever the secrets are: by the dashboard for the controller it runs
 * on, by an agent for its own. Both services listen on loopback only (dainty-lass docs/architecture.md),
 * so neither ever leaves the host it runs on.
 */

export interface GameConfig {
  adminApiUrl: string;
  adminApiKey?: string;
  backendUrl: string;
  operatorSecret?: string;
}

export type GameErrorCode =
  | "not_configured"
  | "unavailable"
  | "refused"
  | "not_found"
  | "forbidden_call"
  | "self_lockout"
  | "invalid_input"
  | "protected_account"
  | "already_banned"
  | "not_banned"
  | "not_timed_out"
  | "timeout"
  | "no_fleet"
  | "not_supported";

export type GameResult<T = unknown> =
  | { ok: true; data: T }
  | { ok: false; code: GameErrorCode; message?: string };

// The admin API's codes (docs/admin-api.md), onto ours.
const ADMIN_CODES: Record<string, GameErrorCode> = {
  BAD_INPUT: "invalid_input",
  NOT_FOUND: "not_found",
  PROTECTED_ACCOUNT: "protected_account",
  ALREADY_BANNED: "already_banned",
  NOT_BANNED: "not_banned",
  NOT_TIMED_OUT: "not_timed_out",
  BACKEND_UNAVAILABLE: "unavailable",
  MODULES_UNAVAILABLE: "unavailable",
  NO_FLEET: "no_fleet",
  NOT_SUPPORTED: "not_supported",
};

const digits = (id: string) => (/^\d{1,32}$/.test(id) ? id : "0");

export function runAdminOp<T>(
  cfg: GameConfig,
  op: AdminOp,
  variables: Record<string, unknown>,
  operatorId: string,
): Promise<GameResult<T>> {
  return runAdminQuery<T>(cfg, op, ADMIN_OPS[op], variables, operatorId);
}

/**
 * A document the caller wrote, whose one root field is `field`. Only for the admin API on the caller's
 * own host: an agent runs ADMIN_OPS by name and nothing else.
 */
export async function runAdminQuery<T>(
  cfg: GameConfig,
  field: string,
  query: string,
  variables: Record<string, unknown>,
  operatorId: string,
): Promise<GameResult<T>> {
  if (!cfg.adminApiKey) return { ok: false, code: "not_configured", message: "admin API key" };
  let response: Response;
  try {
    response = await fetch(cfg.adminApiUrl, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${cfg.adminApiKey}`,
        "x-dainty-operator-id": digits(operatorId),
      },
      body: JSON.stringify({ query, variables }),
      signal: AbortSignal.timeout(15_000),
    });
  } catch {
    return { ok: false, code: "unavailable" };
  }
  const body = (await response.json().catch(() => null)) as {
    data?: Record<string, unknown>;
    errors?: { message: string; extensions?: { code?: string } }[];
  } | null;
  const error = body?.errors?.[0];
  if (error)
    return {
      ok: false,
      code: ADMIN_CODES[error.extensions?.code ?? ""] ?? "refused",
      message: error.message,
    };
  if (!response.ok || !body?.data)
    return { ok: false, code: "refused", message: `HTTP ${response.status}` };
  return { ok: true, data: body.data[field] as T };
}

export function signOperatorRequest(
  secret: string,
  method: string,
  target: string,
  timestamp: string,
  operatorId: string,
  body: string,
): string {
  const bodySha = createHash("sha256").update(body, "utf8").digest("hex");
  const canonical = [method.toUpperCase(), target, timestamp, operatorId, bodySha].join("\n");
  return createHmac("sha256", Buffer.from(secret, "utf8")).update(canonical, "utf8").digest("hex");
}

/** The backend drops control characters from the name; dropping them here keeps the header valid. */
function printable(text: string): string {
  return [...text].filter((c) => c.charCodeAt(0) >= 0x20 && c.charCodeAt(0) !== 0x7f).join("");
}

/** A signed call to the backend's /admin/v1 as the manager operator (backend security/managerOperator.ts). */
export async function runBackend<T>(
  cfg: GameConfig,
  method: string,
  target: string,
  operator: Operator,
  payload?: unknown,
): Promise<GameResult<T>> {
  if (!backendAllowed(method, target))
    return { ok: false, code: "forbidden_call", message: `${method} ${target}` };
  if (!cfg.operatorSecret)
    return { ok: false, code: "not_configured", message: "manager operator secret" };
  const body = payload === undefined ? "" : JSON.stringify(payload);
  const timestamp = String(Date.now());
  const operatorId = digits(operator.id);
  let response: Response;
  try {
    response = await fetch(`${cfg.backendUrl.replace(/\/+$/, "")}${target}`, {
      method,
      headers: {
        ...(body ? { "Content-Type": "application/json" } : {}),
        "X-Paradox-Operator-Id": operatorId,
        "X-Paradox-Operator-Name": printable(operator.name).slice(0, 64),
        "X-Paradox-Operator-Timestamp": timestamp,
        "X-Paradox-Operator-Signature": signOperatorRequest(
          cfg.operatorSecret,
          method,
          target,
          timestamp,
          operatorId,
          body,
        ),
      },
      body: body || undefined,
      signal: AbortSignal.timeout(15_000),
    });
  } catch {
    return { ok: false, code: "unavailable" };
  }
  const json = (await response.json().catch(() => null)) as
    | ({ error?: { code?: string; message?: string } } & T)
    | null;
  if (!response.ok) {
    if (response.status === 404) return { ok: false, code: "not_found" };
    if (json?.error?.code === "ADMIN_SELF_LOCKOUT") return { ok: false, code: "self_lockout" };
    return {
      ok: false,
      code: "refused",
      message: json?.error?.message ?? `HTTP ${response.status}`,
    };
  }
  return { ok: true, data: json as T };
}
