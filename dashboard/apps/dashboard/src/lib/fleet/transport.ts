import {
  type AdminOp,
  type GameConfig,
  type GameResult,
  type Operator,
  runAdminOp,
  runBackend,
} from "@dainty/fleet";
import { config } from "@/lib/config";
import { DomainError, type ErrorCode } from "@/lib/errors";
import { requireController } from "./controllers";
import { isConnected, sendCommand } from "./registry";

/**
 * One way to reach any controller's game: the local one directly with the dashboard's own secrets, any
 * other through its agent, which holds that host's secrets and runs the same calls (@dainty/fleet).
 */

export const localGameConfig = (): GameConfig => ({
  adminApiUrl: config.adminApi.url,
  adminApiKey: config.adminApi.key,
  backendUrl: config.backend.url,
  operatorSecret: config.backend.operatorSecret,
});

const CODES: Record<string, ErrorCode> = {
  not_configured: "game_not_configured",
  unavailable: "game_unavailable",
  refused: "game_refused",
  not_found: "not_found",
  forbidden_call: "forbidden_call",
  self_lockout: "self_lockout",
  invalid_input: "invalid_input",
  protected_account: "protected_account",
  already_banned: "already_banned",
  not_banned: "not_banned",
  not_timed_out: "not_timed_out",
  timeout: "game_timeout",
};

function unwrap<T>(result: GameResult): T {
  if (result.ok) return result.data as T;
  throw new DomainError(CODES[result.code] ?? "game_refused", result.message);
}

export interface GameLink {
  controllerId: string;
  admin<T>(op: AdminOp, variables: Record<string, unknown>, operatorId: string): Promise<T>;
  backend<T>(method: string, target: string, operator: Operator, body?: unknown): Promise<T>;
}

export function game(controllerId: string): GameLink {
  const c = requireController(controllerId);
  if (c.kind === "local") {
    return {
      controllerId,
      admin: async (op, variables, operatorId) =>
        unwrap(await runAdminOp(localGameConfig(), op, variables, operatorId)),
      backend: async (method, target, operator, body) =>
        unwrap(await runBackend(localGameConfig(), method, target, operator, body)),
    };
  }
  const offline = () => {
    if (!isConnected(controllerId)) throw new DomainError("controller_offline", c.name);
  };
  return {
    controllerId,
    admin: async (op, variables, operatorId) => {
      offline();
      return unwrap(await sendCommand(controllerId, { kind: "admin", op, variables, operatorId }));
    },
    backend: async (method, target, operator, body) => {
      offline();
      return unwrap(
        await sendCommand(controllerId, { kind: "backend", method, target, operator, body }),
      );
    },
  };
}
