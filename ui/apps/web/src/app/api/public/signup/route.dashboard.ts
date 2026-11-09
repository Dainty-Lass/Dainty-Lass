import { NextResponse } from "next/server";
import { signupState } from "@/lib/signup";

// The Discord bot reads any failure as off and caches for 60 s (D3).
export function GET() {
  try {
    return NextResponse.json(signupState(), { headers: { "Cache-Control": "public, max-age=60" } });
  } catch {
    return NextResponse.json({ enabled: false, url: null }, { status: 503 });
  }
}
