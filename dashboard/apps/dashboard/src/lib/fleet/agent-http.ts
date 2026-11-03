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

export function pairingLocked(ip: string): boolean {
  const f = failures.get(ip);
  if (!f || Date.now() - f.since > WINDOW_MS) return false;
  return f.count >= MAX_FAILURES;
}

export function pairingFailed(ip: string): void {
  const f = failures.get(ip);
  if (!f || Date.now() - f.since > WINDOW_MS) failures.set(ip, { count: 1, since: Date.now() });
  else f.count++;
}
