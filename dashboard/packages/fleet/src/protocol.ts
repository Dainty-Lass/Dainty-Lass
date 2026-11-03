import type { GameResult } from "./game";
import type { MetricsSummary } from "./metrics";
import type { AdminOp } from "./ops";
import type { HuntStatus, Operator, ServerStatus } from "./types";

/**
 * The agent protocol. The agent dials the dashboard and holds one event stream open; commands come down
 * it, and results, status and relayed chat go back as plain POSTs. The dashboard never dials an agent,
 * so a controller opens no port for it.
 *
 *   POST /api/agent/v1/pair         { code, name, version }          -> { agentId, token }
 *   GET  /api/agent/v1/stream       text/event-stream of AgentCommand ("command" events, "ping" keep-alives)
 *   POST /api/agent/v1/result       AgentResult
 *   POST /api/agent/v1/status       AgentStatus, with the host's /metrics summarised when it answers
 *   POST /api/agent/v1/ingest/chat, /ingest/reports, GET /chat-filter: the ingest API, tagged with the agent
 *
 * Every call but pairing carries Authorization: Bearer <token>.
 */

export const PROTOCOL_VERSION = 1;
export const HEARTBEAT_MS = 15_000;
/** A controller with no status this long is shown as offline. */
export const STALE_MS = 45_000;
export const COMMAND_TIMEOUT_MS = 20_000;
export const PAIRING_TTL_MS = 30 * 60_000;

export type AgentCommand =
  | {
      id: string;
      kind: "admin";
      op: AdminOp;
      variables: Record<string, unknown>;
      operatorId: string;
    }
  | {
      id: string;
      kind: "backend";
      method: string;
      target: string;
      operator: Operator;
      body?: unknown;
    };

export interface AgentResult {
  id: string;
  result: GameResult;
}

export type ServiceState = "ok" | "down" | "not_configured";

export interface AgentStatus {
  protocol: number;
  version: string;
  /** Command kinds this agent runs; the dashboard sends nothing else to it. */
  capabilities: string[];
  at: number;
  adminApi: ServiceState;
  backend: ServiceState;
  server: ServerStatus | null;
  hunts: HuntStatus | null;
  online: number | null;
  /**
   * The admin service's `/metrics`, summarised (metrics.ts). Absent from agents older than the
   * "metrics" capability, null when the host's `/metrics` did not answer.
   */
  metrics?: MetricsSummary | null;
}

export interface PairRequest {
  code: string;
  name: string;
  version: string;
}

export interface PairResponse {
  agentId: string;
  token: string;
}

/** A pairing code as people read and type it: eight letters, no look-alikes, shown as ABCD-EFGH. */
export const PAIRING_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ";

export function normalizePairingCode(code: string): string {
  return code.toUpperCase().replace(/[^A-Z]/g, "");
}
