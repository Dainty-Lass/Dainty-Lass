import { join, resolve } from "node:path";
import { type GameConfig, settledUrl } from "@dainty/fleet";

/**
 * Everything from the environment (`bun --env-file=agent.env ...`). The DAINTY_AGENT_ prefix is one the
 * host kit's `start` can be taught to pass through, as with the other server prefixes.
 */
function env(name: string): string | undefined {
  const v = process.env[`DAINTY_AGENT_${name}`]?.trim();
  return v ? v : undefined;
}

export interface AgentConfig {
  dashboardUrl: string;
  pairingCode?: string;
  tokenFile: string;
  game: GameConfig;
  ingestPort: number;
  /** The port before 2026-11-10, also served while INGEST_PORT is unset, for a backend still pointed at it. */
  legacyIngestPort?: number;
  ingestKey?: string;
  version: string;
}

export function loadConfig(): AgentConfig {
  const dashboardUrl = env("DASHBOARD_URL")?.replace(/\/+$/, "");
  if (!dashboardUrl) throw new Error("DAINTY_AGENT_DASHBOARD_URL is not set");
  const url = new URL(dashboardUrl);
  // The token goes with every request: only loopback may carry it in the clear.
  if (
    url.protocol !== "https:" &&
    !["localhost", "127.0.0.1", "[::1]"].includes(url.hostname) &&
    env("ALLOW_HTTP") !== "on"
  ) {
    throw new Error(
      "DAINTY_AGENT_DASHBOARD_URL must be https (set DAINTY_AGENT_ALLOW_HTTP=on to override on a private network)",
    );
  }
  return {
    dashboardUrl,
    pairingCode: env("PAIRING_CODE"),
    tokenFile: resolve(env("TOKEN_FILE") ?? join(process.cwd(), "data", "agent-token")),
    game: {
      adminApiUrl: settledUrl(
        "DAINTY_AGENT_ADMIN_API_URL",
        env("ADMIN_API_URL") ?? "http://127.0.0.1:41003/graphql",
      ),
      adminApiKey: env("ADMIN_API_KEY"),
      backendUrl: settledUrl(
        "DAINTY_AGENT_BACKEND_URL",
        env("BACKEND_URL") ?? "http://127.0.0.1:41000",
      ),
      operatorSecret: env("OPERATOR_SECRET"),
    },
    // Below the range Windows lends to Hyper-V, WSL and Docker
    ingestPort: Number(env("INGEST_PORT") ?? 41009),
    legacyIngestPort: env("INGEST_PORT") ? undefined : 61009,
    ingestKey: env("INGEST_KEY"),
    version: env("VERSION") ?? "0.1.0",
  };
}
