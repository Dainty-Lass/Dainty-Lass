import { createHash, timingSafeEqual } from "node:crypto";
import type { AgentConfig } from "./config";

/**
 * The ingest API on loopback, for this host's backend: the same routes as the dashboard's /api/ingest,
 * forwarded with the agent's token so the dashboard knows which controller the chat came from. The
 * backend's configuration is the same whether the dashboard is local or not; only the URL differs.
 */

const ROUTES: Record<string, { method: string; upstream: string }> = {
  "/api/ingest/chat": { method: "POST", upstream: "/api/agent/v1/ingest/chat" },
  "/api/ingest/reports": { method: "POST", upstream: "/api/agent/v1/ingest/reports" },
  "/api/ingest/chat-filter": { method: "GET", upstream: "/api/agent/v1/chat-filter" },
};

function keyMatches(given: string, expected: string): boolean {
  const a = createHash("sha256").update(given).digest();
  const b = createHash("sha256").update(expected).digest();
  return timingSafeEqual(a, b);
}

export function startIngestRelay(
  cfg: AgentConfig,
  call: (path: string, init?: RequestInit) => Promise<Response>,
) {
  if (!cfg.ingestKey || cfg.ingestKey.length < 32) {
    console.log(
      "ingest relay off: set DAINTY_AGENT_INGEST_KEY (32+ characters) to relay chat and reports",
    );
    return null;
  }
  const key = cfg.ingestKey;
  let main: ReturnType<typeof serve>;
  try {
    main = serve(cfg.ingestPort, key, call);
  } catch (error) {
    // Commands and status matter more than the relay: keep running without it.
    console.log(
      `ingest relay off: 127.0.0.1:${cfg.ingestPort} is not available (${error instanceof Error ? error.message : error})`,
    );
    return null;
  }
  let legacy: ReturnType<typeof serve> | null = null;
  if (cfg.legacyIngestPort !== undefined) {
    try {
      legacy = serve(cfg.legacyIngestPort, key, call);
      console.log(
        `ingest relay also on 127.0.0.1:${cfg.legacyIngestPort}, the old port: point the backend at ${cfg.ingestPort}`,
      );
    } catch {
      // Taken, or lent out by Windows: the new port is enough
    }
  }
  return {
    stop() {
      main.stop();
      legacy?.stop();
    },
  };
}

function serve(
  port: number,
  key: string,
  call: (path: string, init?: RequestInit) => Promise<Response>,
) {
  return Bun.serve({
    hostname: "127.0.0.1",
    port,
    async fetch(request) {
      const route = ROUTES[new URL(request.url).pathname];
      if (!route || request.method !== route.method) return new Response(null, { status: 404 });
      const given = request.headers.get("authorization")?.replace(/^Bearer\s+/i, "") ?? "";
      if (!keyMatches(given, key)) return Response.json({ error: "unauthorized" }, { status: 401 });
      try {
        const body = route.method === "POST" ? await request.text() : undefined;
        if (body && body.length > 2_000_000)
          return Response.json({ error: "too_large" }, { status: 413 });
        const upstream = await call(route.upstream, {
          method: route.method,
          headers: {
            ...(body ? { "Content-Type": "application/json" } : {}),
            ...(request.headers.get("if-none-match")
              ? { "If-None-Match": request.headers.get("if-none-match")! }
              : {}),
          },
          body,
          signal: AbortSignal.timeout(15_000),
        });
        return new Response(upstream.body, {
          status: upstream.status,
          headers: {
            "Content-Type": upstream.headers.get("content-type") ?? "application/json",
            ETag: upstream.headers.get("etag") ?? "",
          },
        });
      } catch {
        // The backend retries; the dashboard being away is not this host's error to keep.
        return Response.json({ error: "dashboard_unreachable" }, { status: 502 });
      }
    },
  });
}
