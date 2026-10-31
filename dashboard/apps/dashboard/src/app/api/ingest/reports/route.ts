import { NextResponse } from "next/server";
import { ingestAuthorized, parseReport } from "@/lib/ingest";
import { createReport } from "@/lib/reports/store";

/** POST one report. The sender's `id` makes a retry return the first report instead of filing it twice. */
export async function POST(request: Request) {
  if (!ingestAuthorized(request))
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  const parsed = parseReport(await request.json().catch(() => null));
  if (!parsed) return NextResponse.json({ error: "bad_request" }, { status: 400 });
  const { report, duplicate } = createReport({ ingest: parsed.source }, parsed);
  return NextResponse.json(
    { id: report.id, status: report.status, duplicate },
    { status: duplicate ? 200 : 201 },
  );
}
