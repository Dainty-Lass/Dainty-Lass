import { NextResponse } from "next/server";
import { ingestChat, pruneIfDue } from "@/lib/chat/store";
import { ingestAuthorized, parseChat } from "@/lib/ingest";
import { getSetting } from "@/lib/settings";

/** POST { messages: [...] }, at most 500 a batch. A line with an id already stored is skipped, so a retry is safe. */
export async function POST(request: Request) {
  if (!ingestAuthorized(request))
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  const body = (await request.json().catch(() => null)) as { messages?: unknown[] } | null;
  const raw = Array.isArray(body?.messages) ? body.messages : null;
  if (!raw || raw.length > 500) return NextResponse.json({ error: "bad_request" }, { status: 400 });
  const parsed = raw.map(parseChat);
  const valid = parsed.filter((m) => m !== null);
  const result = ingestChat(valid);
  pruneIfDue(getSetting("chatRetentionDays"));
  return NextResponse.json({ ...result, rejected: parsed.length - valid.length });
}
