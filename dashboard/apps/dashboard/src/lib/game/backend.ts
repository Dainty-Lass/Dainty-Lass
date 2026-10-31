import { createHash, createHmac } from "node:crypto";
import { config } from "@/lib/config";
import { DomainError } from "@/lib/errors";

/**
 * The backend's /admin/v1, called as the manager operator: the contract in the backend's
 * security/managerOperator.ts. Each call is signed for the staff member's Discord id, so the backend's
 * own audit trail names them as manager:<id> without a backend session.
 */

export interface BackendAccount {
  userId: string;
  displayName: string;
  email: string;
  approvalStatus: "pending" | "approved" | "rejected";
  status: "active" | "disabled" | "banned";
  roles: string[];
  rolesUpdatedAt?: string;
  rolesUpdatedBy?: string;
  usernameSet: boolean;
  createdAt: string;
  lastLoginAt?: string;
  approvalUpdatedAt?: string;
  approvalUpdatedBy?: string;
  approvalReason?: string;
}

export interface BackendAuditRecord {
  id?: string;
  action: string;
  actor?: string;
  actorUserId?: string;
  targetUserId?: string;
  reason?: string;
  createdAt?: string;
  oldState?: unknown;
  newState?: unknown;
}

export function backendConfigured(): boolean {
  return Boolean(config.backend.operatorSecret);
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

async function call<T>(
  method: string,
  target: string,
  operator: { id: string; name: string },
  payload?: unknown,
): Promise<T> {
  const secret = config.backend.operatorSecret;
  if (!secret) throw new DomainError("game_not_configured", "DASHBOARD_MANAGER_OPERATOR_SECRET");
  const body = payload === undefined ? "" : JSON.stringify(payload);
  const timestamp = String(Date.now());
  const operatorId = /^\d{1,32}$/.test(operator.id) ? operator.id : "0";
  let response: Response;
  try {
    response = await fetch(`${config.backend.url}${target}`, {
      method,
      headers: {
        ...(body ? { "Content-Type": "application/json" } : {}),
        "X-Paradox-Operator-Id": operatorId,
        "X-Paradox-Operator-Name": printable(operator.name).slice(0, 64),
        "X-Paradox-Operator-Timestamp": timestamp,
        "X-Paradox-Operator-Signature": signOperatorRequest(
          secret,
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
    throw new DomainError("game_unavailable");
  }
  const json = (await response.json().catch(() => null)) as {
    error?: { code?: string; message?: string };
  } | null;
  if (!response.ok) {
    const code = json?.error?.code;
    if (response.status === 404) throw new DomainError("not_found");
    if (code === "ADMIN_SELF_LOCKOUT") throw new DomainError("self_lockout");
    throw new DomainError("game_refused", json?.error?.message ?? `HTTP ${response.status}`);
  }
  return json as T;
}

export const backend = {
  players: (
    q: { search?: string; status?: string; page?: number; limit?: number },
    op: { id: string; name: string },
  ) => {
    const params = new URLSearchParams();
    if (q.search) params.set("search", q.search);
    if (q.status) params.set("status", q.status);
    params.set("page", String(q.page ?? 1));
    params.set("limit", String(q.limit ?? 25));
    return call<{ accounts: BackendAccount[]; total: number; page: number; limit: number }>(
      "GET",
      `/admin/v1/players?${params}`,
      op,
    );
  },
  playersById: (ids: string[], op: { id: string; name: string }) =>
    call<{ accounts: BackendAccount[] }>(
      "GET",
      `/admin/v1/players?ids=${ids.map(encodeURIComponent).join(",")}`,
      op,
    ),
  onlinePlayers: (op: { id: string; name: string }) =>
    call<{ count: number; players: { userId: string; displayName: string }[]; observedAt: string }>(
      "GET",
      "/admin/v1/online-players",
      op,
    ),
  setAccess: (
    userId: string,
    change: {
      status?: BackendAccount["status"];
      approvalStatus?: BackendAccount["approvalStatus"];
      reason: string;
    },
    op: { id: string; name: string },
  ) =>
    call<{ account: BackendAccount }>(
      "PATCH",
      `/admin/v1/players/${encodeURIComponent(userId)}/access`,
      op,
      change,
    ),
  setRoles: (userId: string, roles: string[], reason: string, op: { id: string; name: string }) =>
    call<{ account: BackendAccount }>(
      "PATCH",
      `/admin/v1/players/${encodeURIComponent(userId)}/roles`,
      op,
      { roles, reason },
    ),
  audit: (
    q: { targetUserId?: string; page?: number; limit?: number },
    op: { id: string; name: string },
  ) => {
    const params = new URLSearchParams({ page: String(q.page ?? 1), limit: String(q.limit ?? 50) });
    if (q.targetUserId) params.set("targetUserId", q.targetUserId);
    return call<{ records: BackendAuditRecord[]; total: number }>(
      "GET",
      `/admin/v1/audit?${params}`,
      op,
    );
  },
};
