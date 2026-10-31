import { cookies } from "next/headers";
import { NextResponse } from "next/server";
import { recordAudit } from "@/lib/audit/record";
import { currentStaff, destroyCurrentSession, SESSION_COOKIE } from "@/lib/auth/session";
import { config } from "@/lib/config";

// POST only: a GET sign-out could be fired by any image tag on another site.
export async function POST() {
  const me = await currentStaff();
  await destroyCurrentSession();
  (await cookies()).delete(SESSION_COOKIE);
  if (me)
    recordAudit(me, {
      action: "session.sign_out",
      targetType: "session",
      targetId: me.id,
      targetLabel: me.username,
    });
  return NextResponse.redirect(`${config.baseUrl}/login`, { status: 303 });
}
