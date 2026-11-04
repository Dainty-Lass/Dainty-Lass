import { NextResponse } from "next/server";
import type { Controller } from "@/lib/db/schema";
import { authenticateAgent } from "./controllers";

/** Runs an agent route as the agent its token names; anything else is a 401 that says nothing more. */
export async function asAgent(
  request: Request,
  handler: (agent: Controller) => Promise<Response> | Response,
): Promise<Response> {
  const agent = authenticateAgent(request);
  if (agent?.kind !== "agent") return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  return handler(agent);
}

// Wrong pairing codes per address, kept in memory: the dashboard is a single process.
const failures = new Map<string, { count: number; since: number }>();
const WINDOW_MS = 15 * 60_000;
const MAX_FAILURES = 10;
// Beyond this many addresses the oldest window goes: the map cannot grow without bound. No overall lock, which
// anyone could trip: a client that varies its address still faces 24^8 codes that expire in 30 minutes.
const MAX_ADDRESSES = 10_000;

const expired = (since: number, now: number) => now - since > WINDOW_MS;

export function pairingLocked(ip: string, now = Date.now()): boolean {
  const f = failures.get(ip);
  if (!f || expired(f.since, now)) return false;
  return f.count >= MAX_FAILURES;
}

export function pairingFailed(ip: string, now = Date.now()): void {
  const f = failures.get(ip);
  if (f && !expired(f.since, now)) {
    f.count++;
    return;
  }
  failures.delete(ip);
  for (const [key, entry] of failures) {
    if (failures.size < MAX_ADDRESSES && !expired(entry.since, now)) break;
    failures.delete(key);
  }
  failures.set(ip, { count: 1, since: now });
}

/** For tests: forget every failure. */
export function resetPairingFailures(): void {
  failures.clear();
}
