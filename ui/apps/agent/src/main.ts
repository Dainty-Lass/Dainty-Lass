import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { collectStatus, HEARTBEAT_MS, type PairResponse } from "@dainty/fleet";
import { type AgentConfig, loadConfig } from "./config";
import { startIngestRelay } from "./relay";
import { runCommand } from "./run";
import { readEvents } from "./sse";

/**
 * The agent: pairs once, then keeps one event stream open to the dashboard for commands, reports status
 * every HEARTBEAT_MS, and relays this host's chat and reports. It opens no port beyond loopback.
 *
 * @deprecated dainty-remote replaces it (D1, module-sharing plan MS8); it gains no new fields.
 */

const log = (...args: unknown[]) => console.log(new Date().toISOString(), ...args);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

class Unauthorized extends Error {}

function readToken(cfg: AgentConfig): string | null {
  return existsSync(cfg.tokenFile) ? readFileSync(cfg.tokenFile, "utf8").trim() || null : null;
}

async function pair(cfg: AgentConfig): Promise<string> {
  if (!cfg.pairingCode)
    throw new Error("not paired: set DAINTY_AGENT_PAIRING_CODE to the code the dashboard showed");
  const response = await fetch(`${cfg.dashboardUrl}/api/agent/v1/pair`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ code: cfg.pairingCode, name: "", version: cfg.version }),
    signal: AbortSignal.timeout(15_000),
  });
  if (!response.ok)
    throw new Error(`pairing refused (${response.status}): the code is wrong, used or expired`);
  const { token, agentId } = (await response.json()) as PairResponse;
  mkdirSync(dirname(cfg.tokenFile), { recursive: true });
  writeFileSync(cfg.tokenFile, token, { mode: 0o600 });
  try {
    chmodSync(cfg.tokenFile, 0o600);
  } catch {
    // Windows has no modes; the file's folder permissions are what protect it there.
  }
  log(`paired as ${agentId}; the pairing code can be removed from the env file`);
  return token;
}

function client(cfg: AgentConfig, token: string) {
  return async (path: string, init: RequestInit = {}) => {
    const response = await fetch(`${cfg.dashboardUrl}${path}`, {
      ...init,
      headers: { ...(init.headers ?? {}), Authorization: `Bearer ${token}` },
    });
    if (response.status === 401) throw new Unauthorized("the dashboard refused this agent's token");
    return response;
  };
}

async function statusLoop(cfg: AgentConfig, call: ReturnType<typeof client>, stop: AbortSignal) {
  while (!stop.aborted) {
    try {
      const status = await collectStatus(cfg.game, cfg.version);
      await call("/api/agent/v1/status", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(status),
        signal: AbortSignal.timeout(10_000),
      });
    } catch (error) {
      if (error instanceof Unauthorized) throw error;
      log("status not sent:", error instanceof Error ? error.message : error);
    }
    await sleep(HEARTBEAT_MS);
  }
}

async function streamLoop(cfg: AgentConfig, call: ReturnType<typeof client>, stop: AbortSignal) {
  let backoff = 1_000;
  while (!stop.aborted) {
    const idle = new AbortController();
    let timer = setTimeout(() => idle.abort(), 60_000);
    try {
      const response = await call("/api/agent/v1/stream", {
        headers: { Accept: "text/event-stream" },
        signal: idle.signal,
      });
      if (!response.ok || !response.body) throw new Error(`stream answered ${response.status}`);
      log("connected");
      backoff = 1_000;
      for await (const { event, data } of readEvents(response.body)) {
        clearTimeout(timer);
        timer = setTimeout(() => idle.abort(), 60_000);
        if (event !== "command") continue;
        // Commands run side by side: a slow ban must not hold up a status read.
        void (async () => {
          const answer = await runCommand(cfg.game, JSON.parse(data));
          if (!answer) return;
          await call("/api/agent/v1/result", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(answer),
          }).catch((e) => log("result not sent:", e instanceof Error ? e.message : e));
        })();
      }
      log("stream closed; reconnecting");
    } catch (error) {
      if (error instanceof Unauthorized) throw error;
      log("stream lost:", error instanceof Error ? error.message : error);
    } finally {
      clearTimeout(timer);
    }
    await sleep(backoff + Math.random() * 500);
    backoff = Math.min(backoff * 2, 30_000);
  }
}

async function main() {
  const cfg = loadConfig();
  log(`dainty agent ${cfg.version} → ${cfg.dashboardUrl}`);
  if (!cfg.game.adminApiKey)
    log("warning: DAINTY_AGENT_ADMIN_API_KEY is unset; moderation and status will fail");
  if (!cfg.game.operatorSecret)
    log(
      "warning: DAINTY_AGENT_OPERATOR_SECRET is unset; player lists and access changes will fail",
    );
  let token = readToken(cfg) ?? (await pair(cfg));
  for (;;) {
    const call = client(cfg, token);
    const stop = new AbortController();
    const relay = startIngestRelay(cfg, call);
    try {
      await Promise.race([statusLoop(cfg, call, stop.signal), streamLoop(cfg, call, stop.signal)]);
    } catch (error) {
      if (!(error instanceof Unauthorized)) throw error;
      stop.abort();
      relay?.stop();
      // Removed or re-paired on the dashboard: a fresh code, if one is configured, pairs again.
      log("token refused:", error.message);
      if (!cfg.pairingCode) {
        log("waiting; re-pair from the dashboard and set DAINTY_AGENT_PAIRING_CODE");
        await sleep(60_000);
        continue;
      }
      token = await pair(cfg).catch(async (e) => {
        log(e instanceof Error ? e.message : e);
        await sleep(60_000);
        return token;
      });
    }
  }
}

void main().catch((error) => {
  log("fatal:", error instanceof Error ? error.message : error);
  process.exit(1);
});
