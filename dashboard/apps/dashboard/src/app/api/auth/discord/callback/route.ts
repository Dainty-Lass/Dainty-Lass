import { timingSafeEqual } from "node:crypto";
import { cookies, headers } from "next/headers";
import { NextResponse } from "next/server";
import { recordAudit } from "@/lib/audit/record";
import { exchangeCode, fetchGuildRoles, fetchUser, roleFromGuild } from "@/lib/auth/discord";
import { OAUTH_COOKIE } from "@/lib/auth/oauth-cookie";
import { clientIp, createSession, sessionCookie } from "@/lib/auth/session";
import { config } from "@/lib/config";
import { getSettings } from "@/lib/settings";
import { resolveSignIn } from "@/lib/staff";

function fail(reason: string) {
  return NextResponse.redirect(`${config.baseUrl}/login?error=${reason}`);
}

function same(a: string, b: string) {
  const x = Buffer.from(a);
  const y = Buffer.from(b);
  return x.length === y.length && timingSafeEqual(x, y);
}

export async function GET(request: Request) {
  const url = new URL(request.url);
  const jar = await cookies();
  const [state, verifier] = (jar.get(OAUTH_COOKIE)?.value ?? "").split(".");
  jar.delete({ name: OAUTH_COOKIE, path: "/api/auth/discord" });
  if (url.searchParams.get("error")) return fail("discord_denied");
  const code = url.searchParams.get("code");
  if (!code || !state || !verifier || !same(state, url.searchParams.get("state") ?? ""))
    return fail("state_mismatch");

  try {
    const token = await exchangeCode(code, verifier);
    const user = await fetchUser(token);
    const settings = getSettings();
    const guildRole = settings.discordGuildId
      ? roleFromGuild(
          (await fetchGuildRoles(token, settings.discordGuildId)) ?? [],
          settings.discordRoleMap,
        )
      : null;
    const outcome = resolveSignIn(user, guildRole);
    const ip = clientIp(await headers());
    if (!outcome.ok) {
      recordAudit(
        { ingest: "discord" },
        {
          action: "session.refused",
          targetType: "session",
          targetId: user.id,
          targetLabel: user.username,
          reason: outcome.reason,
          ip,
        },
      );
      return fail(outcome.reason);
    }
    const { token: session, expiresAt } = await createSession(outcome.staff.id);
    recordAudit(outcome.staff, {
      action: "session.sign_in",
      targetType: "session",
      targetId: outcome.staff.id,
      targetLabel: outcome.staff.username,
      ip,
    });
    jar.set(sessionCookie(session, expiresAt));
    return NextResponse.redirect(`${config.baseUrl}/`);
  } catch (error) {
    // The message only: Discord's answers can echo the code, which is a credential for a few minutes.
    console.error(
      "[auth] Discord sign-in failed:",
      error instanceof Error ? error.message : "unknown",
    );
    return fail("discord_failed");
  }
}
