import { NextResponse } from "next/server";
import { filterVersion, listTerms } from "@/lib/chat/terms";
import { asAgent } from "@/lib/fleet/agent-http";

/** One filter for the whole fleet: every agent hands its backend the same list. */
export function GET(request: Request) {
  return asAgent(request, () => {
    const version = filterVersion();
    const etag = `"filter-${version}"`;
    if (request.headers.get("if-none-match") === etag)
      return new NextResponse(null, { status: 304, headers: { ETag: etag } });
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
  });
}
