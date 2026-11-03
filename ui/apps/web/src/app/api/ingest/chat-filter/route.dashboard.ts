import { NextResponse } from "next/server";
import { filterVersion, listTerms } from "@/lib/chat/terms";
import { ingestAuthorized } from "@/lib/ingest";

/** The filter for the backend to enforce. The version is the ETag, so a poll that finds no change costs a 304. */
export async function GET(request: Request) {
  if (!ingestAuthorized(request))
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  const version = filterVersion();
  const etag = `"filter-${version}"`;
  if (request.headers.get("if-none-match") === etag) {
    return new NextResponse(null, { status: 304, headers: { ETag: etag } });
  }
  const terms = listTerms()
    .filter((t) => t.enabled)
    .map(({ id, pattern, matchType, action, channels }) => ({
      id,
      pattern,
      matchType,
      action,
      channels,
    }));
  return NextResponse.json(
    { version, terms },
    { headers: { ETag: etag, "Cache-Control": "no-cache" } },
  );
}
