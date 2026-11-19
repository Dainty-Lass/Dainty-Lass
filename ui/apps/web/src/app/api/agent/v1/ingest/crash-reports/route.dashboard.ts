import { NextResponse } from "next/server";
import { parseCrashReport } from "@/lib/crash-reports/parse";
import { createCrashReport } from "@/lib/crash-reports/store";
import { asAgent } from "@/lib/fleet/agent-http";

export function POST(request: Request) {
  return asAgent(request, async (agent) => {
    const parsed = parseCrashReport(await request.json().catch(() => null));
    if (!parsed) return NextResponse.json({ error: "bad_request" }, { status: 400 });
    const { report, duplicate } = createCrashReport(parsed, agent.id);
    return NextResponse.json({ id: report.id, duplicate }, { status: duplicate ? 200 : 201 });
  });
}
