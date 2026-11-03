import type { BackendAccount, Operator } from "@dainty/fleet";
import { config } from "@/lib/config";
import { game } from "@/lib/fleet/transport";

/**
 * The backend's /admin/v1 on one controller, called as the manager operator (backend
 * security/managerOperator.ts): each call is signed for the staff member's Discord id, so the backend's
 * own audit names them as manager:<id>. Remote calls are signed by that controller's agent.
 */

export type { BackendAccount } from "@dainty/fleet";

export interface BackendAuditRecord {
  id?: string;
  action: string;
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

export function backend(controllerId: string) {
  const g = game(controllerId);
  return {
    players: (
      q: { search?: string; status?: string; page?: number; limit?: number },
      op: Operator,
    ) => {
      const params = new URLSearchParams();
      if (q.search) params.set("search", q.search);
      if (q.status) params.set("status", q.status);
      params.set("page", String(q.page ?? 1));
      params.set("limit", String(q.limit ?? 25));
      return g.backend<{ accounts: BackendAccount[]; total: number; page: number; limit: number }>(
        "GET",
        `/admin/v1/players?${params}`,
        op,
      );
    },
    playersById: (ids: string[], op: Operator) =>
      g.backend<{ accounts: BackendAccount[] }>(
        "GET",
        `/admin/v1/players?ids=${ids.map(encodeURIComponent).join(",")}`,
        op,
      ),
    onlinePlayers: (op: Operator) =>
      g.backend<{
        count: number;
        players: { userId: string; displayName: string }[];
        observedAt: string;
      }>("GET", "/admin/v1/online-players", op),
    setAccess: (
      userId: string,
      change: {
        status?: BackendAccount["status"];
        approvalStatus?: BackendAccount["approvalStatus"];
        reason: string;
      },
      op: Operator,
    ) =>
      g.backend<{ account: BackendAccount }>(
        "PATCH",
        `/admin/v1/players/${encodeURIComponent(userId)}/access`,
        op,
        change,
      ),
    setRoles: (userId: string, roles: string[], reason: string, op: Operator) =>
      g.backend<{ account: BackendAccount }>(
        "PATCH",
        `/admin/v1/players/${encodeURIComponent(userId)}/roles`,
        op,
        { roles, reason },
      ),
    audit: (q: { targetUserId?: string; page?: number; limit?: number }, op: Operator) => {
      const params = new URLSearchParams({
        page: String(q.page ?? 1),
        limit: String(q.limit ?? 50),
      });
      if (q.targetUserId) params.set("targetUserId", q.targetUserId);
      return g.backend<{ records: BackendAuditRecord[]; total: number }>(
        "GET",
        `/admin/v1/audit?${params}`,
        op,
      );
    },
  };
}
