import { NextResponse } from "next/server";
import { ingestChat, pruneIfDue } from "@/lib/chat/store";
import { asAgent } from "@/lib/fleet/agent-http";
import { parseChat } from "@/lib/ingest";
import { getSetting } from "@/lib/settings";

/** The ingest API relayed by an agent: the same body as /api/ingest/chat, tagged with the agent's controller. */
export function POST(request: Request) {
  return asAgent(request, async (agent) => {
    const body = (await request.json().catch(() => null)) as { messages?: unknown[] } | null;
    const raw = Array.isArray(body?.messages) ? body.messages : null;
    if (!raw || raw.length > 500)
      return NextResponse.json({ error: "bad_request" }, { status: 400 });
    const parsed = raw.map(parseChat);
    const valid = parsed.filter((m) => m !== null);
    const result = ingestChat(valid, agent.id);
    pruneIfDue(getSetting("chatRetentionDays"));
    return NextResponse.json({ ...result, rejected: parsed.length - valid.length });
  });
}
