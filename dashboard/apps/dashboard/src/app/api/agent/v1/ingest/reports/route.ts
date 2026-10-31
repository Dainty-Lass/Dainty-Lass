import { NextResponse } from "next/server";
import { asAgent } from "@/lib/fleet/agent-http";
import { parseReport } from "@/lib/ingest";
import { createReport } from "@/lib/reports/store";

export function POST(request: Request) {
  return asAgent(request, async (agent) => {
    const parsed = parseReport(await request.json().catch(() => null));
    if (!parsed) return NextResponse.json({ error: "bad_request" }, { status: 400 });
    const { report, duplicate } = createReport(
      { ingest: agent.name },
      { ...parsed, controllerId: agent.id },
    );
    return NextResponse.json(
      { id: report.id, status: report.status, duplicate },
      { status: duplicate ? 200 : 201 },
    );
  });
}
