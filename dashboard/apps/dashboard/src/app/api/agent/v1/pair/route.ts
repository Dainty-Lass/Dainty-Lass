import type { PairRequest } from "@dainty/fleet";
import { headers } from "next/headers";
import { NextResponse } from "next/server";
import { clientIp } from "@/lib/auth/session";
import { pairingFailed, pairingLocked } from "@/lib/fleet/agent-http";
import { pairAgent } from "@/lib/fleet/controllers";

/** The one agent route without a token: it is how an agent gets one, from a single-use code. */
export async function POST(request: Request) {
  const ip = clientIp(await headers()) ?? "unknown";
  if (pairingLocked(ip)) return NextResponse.json({ error: "rate_limited" }, { status: 429 });
  const body = (await request.json().catch(() => null)) as Partial<PairRequest> | null;
  if (typeof body?.code !== "string" || typeof body.version !== "string") {
    return NextResponse.json({ error: "bad_request" }, { status: 400 });
  }
  const paired = pairAgent(body.code, body.version);
  if (!paired) {
    pairingFailed(ip);
    return NextResponse.json({ error: "invalid_code" }, { status: 403 });
  }
  return NextResponse.json(paired);
}
