import { cookies } from "next/headers";
import { NextResponse } from "next/server";
import { recordAudit } from "@/lib/audit/record";
import { createSession, sessionCookie } from "@/lib/auth/session";
import { config } from "@/lib/config";
import { resolveSignIn } from "@/lib/staff";

/**
 * DASHBOARD_DEV_LOGIN=on outside production: signs in as the first id in DASHBOARD_OWNER_DISCORD_IDS,
 * for working on the dashboard without a Discord application.
 */
export async function POST() {
  const id = config.ownerDiscordIds[0];
  if (!config.devLogin || !id) return new NextResponse(null, { status: 404 });
  const outcome = resolveSignIn(
    { id, username: "dev-owner", global_name: "Dev Owner", avatar: null },
    null,
  );
  if (!outcome.ok)
    return NextResponse.redirect(`${config.baseUrl}/login?error=${outcome.reason}`, {
      status: 303,
    });
  const { token, expiresAt } = await createSession(outcome.staff.id);
  recordAudit(outcome.staff, {
    action: "session.sign_in",
    targetType: "session",
    targetId: outcome.staff.id,
    targetLabel: "dev",
  });
  (await cookies()).set(sessionCookie(token, expiresAt));
  return NextResponse.redirect(`${config.baseUrl}/`, { status: 303 });
}
