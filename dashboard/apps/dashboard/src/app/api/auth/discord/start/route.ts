import { cookies } from "next/headers";
import { NextResponse } from "next/server";
import { beginAuthorization } from "@/lib/auth/discord";
import { OAUTH_COOKIE } from "@/lib/auth/oauth-cookie";
import { config, discordConfigured, secureCookies } from "@/lib/config";
import { getSetting } from "@/lib/settings";

export async function GET() {
  if (!discordConfigured())
    return NextResponse.redirect(`${config.baseUrl}/login?error=discord_not_configured`);
  const { url, state, verifier } = beginAuthorization(Boolean(getSetting("discordGuildId")));
  (await cookies()).set({
    name: OAUTH_COOKIE,
    value: `${state}.${verifier}`,
    httpOnly: true,
    secure: secureCookies(),
    // Lax: Discord sends the person back with a top-level GET, which Strict would strip it from.
    sameSite: "lax",
    path: "/api/auth/discord",
    maxAge: 600,
  });
  return NextResponse.redirect(url);
}
