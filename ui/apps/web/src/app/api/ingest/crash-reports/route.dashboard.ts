import { NextResponse } from "next/server";
import { parseCrashReport } from "@/lib/crash-reports/parse";
import { createCrashReport } from "@/lib/crash-reports/store";
import { ingestAuthorized } from "@/lib/ingest";

/** POST one crash report from the content service; its `id` makes a retry a duplicate, not a second row. */
export async function POST(request: Request) {
  if (!ingestAuthorized(request))
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  const parsed = parseCrashReport(await request.json().catch(() => null));
  if (!parsed) return NextResponse.json({ error: "bad_request" }, { status: 400 });
  const { report, duplicate } = createCrashReport(parsed);
  return NextResponse.json({ id: report.id, duplicate }, { status: duplicate ? 200 : 201 });
}
