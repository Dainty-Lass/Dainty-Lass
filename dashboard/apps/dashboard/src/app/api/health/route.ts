import { NextResponse } from "next/server";
import { db, schema } from "@/lib/db";

export async function GET() {
  try {
    db.select({ key: schema.settings.key }).from(schema.settings).limit(1).all();
    return NextResponse.json({ ok: true });
  } catch {
    return NextResponse.json({ ok: false }, { status: 503 });
  }
}
