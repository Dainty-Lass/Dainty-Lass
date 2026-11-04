import type { AgentResult } from "@dainty/fleet";
import { NextResponse } from "next/server";
import { asAgent } from "@/lib/fleet/agent-http";
import { settle } from "@/lib/fleet/registry";

export function POST(request: Request) {
  return asAgent(request, async (agent) => {
    const body = (await request.json().catch(() => null)) as Partial<AgentResult> | null;
    if (typeof body?.id !== "string" || !body.result || typeof body.result.ok !== "boolean") {
      return NextResponse.json({ error: "bad_request" }, { status: 400 });
    }
    // A late or foreign answer is dropped; the command already timed out or belongs to another agent.
    return NextResponse.json({
      accepted: settle(agent.id, body.id, body.result as AgentResult["result"]),
    });
  });
}
