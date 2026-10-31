import { randomUUID } from "node:crypto";
import { type AgentCommand, COMMAND_TIMEOUT_MS, type GameResult } from "@dainty/fleet";

/**
 * The agents connected to this process, and the commands waiting on them. The dashboard is one process
 * on one controller, so this map is the whole truth: an agent is reachable exactly when it is here.
 */

interface Link {
  send: (event: string, data: unknown) => void;
  close: () => void;
  connectedAt: number;
}

interface Pending {
  controllerId: string;
  resolve: (r: GameResult) => void;
  timer: ReturnType<typeof setTimeout>;
}

type DistributiveOmit<T, K extends keyof never> = T extends unknown ? Omit<T, K> : never;

type State = { links: Map<string, Link>; pending: Map<string, Pending> };
// Kept on globalThis so a dev reload does not orphan the streams already open.
const holder = globalThis as unknown as { __daintyFleet?: State };
holder.__daintyFleet ??= { links: new Map(), pending: new Map() };
const state: State = holder.__daintyFleet;

export function attach(controllerId: string, link: Omit<Link, "connectedAt">): () => void {
  // A second stream from the same agent replaces the first: the old one is a dead connection not yet noticed.
  state.links.get(controllerId)?.close();
  const entry = { ...link, connectedAt: Date.now() };
  state.links.set(controllerId, entry);
  return () => {
    if (state.links.get(controllerId) !== entry) return;
    state.links.delete(controllerId);
    for (const [id, p] of state.pending) {
      if (p.controllerId !== controllerId) continue;
      clearTimeout(p.timer);
      state.pending.delete(id);
      p.resolve({ ok: false, code: "unavailable", message: "agent disconnected" });
    }
  };
}

export function isConnected(controllerId: string): boolean {
  return state.links.has(controllerId);
}

export function connectedSince(controllerId: string): number | null {
  return state.links.get(controllerId)?.connectedAt ?? null;
}

export function sendCommand(
  controllerId: string,
  command: DistributiveOmit<AgentCommand, "id">,
): Promise<GameResult> {
  const link = state.links.get(controllerId);
  if (!link)
    return Promise.resolve({ ok: false, code: "unavailable", message: "agent not connected" });
  const id = randomUUID();
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      state.pending.delete(id);
      resolve({ ok: false, code: "timeout" });
    }, COMMAND_TIMEOUT_MS);
    state.pending.set(id, { controllerId, resolve, timer });
    link.send("command", { ...command, id });
  });
}

/** An agent's answer. One from another agent than the command went to is ignored. */
export function settle(controllerId: string, id: string, result: GameResult): boolean {
  const p = state.pending.get(id);
  if (!p || p.controllerId !== controllerId) return false;
  clearTimeout(p.timer);
  state.pending.delete(id);
  p.resolve(result);
  return true;
}
