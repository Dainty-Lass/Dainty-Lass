import { type GameConfig, runAdminOp, runBackend } from "./game";
import { scrapeMetrics } from "./metrics";
import { type AgentStatus, PROTOCOL_VERSION, type ServiceState } from "./protocol";
import type { HuntStatus, ServerStatus } from "./types";

/** One controller's report, the same whether the dashboard makes it for its own host or an agent does. */
export async function collectStatus(cfg: GameConfig, version: string): Promise<AgentStatus> {
  const [server, hunts, online, metrics] = await Promise.all([
    runAdminOp<ServerStatus>(cfg, "serverStatus", {}, "0"),
    runAdminOp<HuntStatus>(cfg, "huntStatus", {}, "0"),
    runBackend<{ count: number }>(cfg, "GET", "/admin/v1/online-players", {
      id: "0",
      name: "status",
    }),
    scrapeMetrics(cfg),
  ]);
  const state = (r: { ok: boolean; code?: string }): ServiceState =>
    r.ok ? "ok" : r.code === "not_configured" ? "not_configured" : "down";
  return {
    protocol: PROTOCOL_VERSION,
    version,
    capabilities: ["admin", "backend", "metrics"],
    at: Date.now(),
    adminApi: state(server),
    backend: state(online),
    server: server.ok ? server.data : null,
    hunts: hunts.ok ? hunts.data : null,
    // The backend counts games that reported in without a chat socket too; prefer it.
    online: online.ok ? online.data.count : server.ok ? server.data.onlineCount : null,
    metrics,
  };
}
