import type { AgentStatus } from "@dainty/fleet";
import { NextResponse } from "next/server";
import { asAgent } from "@/lib/fleet/agent-http";
import { recordStatus } from "@/lib/fleet/status";

const STATES = new Set(["ok", "down", "not_configured"]);

export function POST(request: Request) {
  return asAgent(request, async (agent) => {
    const body = (await request.json().catch(() => null)) as AgentStatus | null;
    if (
      !body ||
      typeof body.version !== "string" ||
      !STATES.has(body.adminApi) ||
      !STATES.has(body.backend)
    ) {
      return NextResponse.json({ error: "bad_request" }, { status: 400 });
    }
    // The dashboard's clock, not the agent's: a host with a wrong clock must not look fresh or stale.
    recordStatus(agent.id, {
      ...body,
      at: Date.now(),
      capabilities: Array.isArray(body.capabilities) ? body.capabilities.slice(0, 20) : [],
    });
    return NextResponse.json({ ok: true });
  });
}
