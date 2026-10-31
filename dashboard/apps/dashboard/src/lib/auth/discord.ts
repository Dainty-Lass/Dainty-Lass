import { createHash, randomBytes } from "node:crypto";
import { config } from "@/lib/config";
import { type Role, rank } from "./permissions";

const API = "https://discord.com/api/v10";

export interface DiscordUser {
  id: string;
  username: string;
  global_name: string | null;
  avatar: string | null;
}

/** State and PKCE verifier travel in one short-lived cookie; the callback must present both. */
export function beginAuthorization(withGuild: boolean): {
  url: string;
  state: string;
  verifier: string;
} {
  const state = randomBytes(24).toString("base64url");
  const verifier = randomBytes(48).toString("base64url");
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  const url = new URL("https://discord.com/oauth2/authorize");
  url.searchParams.set("client_id", config.discord.clientId ?? "");
  url.searchParams.set("response_type", "code");
  url.searchParams.set("redirect_uri", redirectUri());
  // guilds.members.read only when roles grant access: asking for more than we use reads as overreach.
  url.searchParams.set("scope", withGuild ? "identify guilds.members.read" : "identify");
  url.searchParams.set("state", state);
  url.searchParams.set("code_challenge", challenge);
  url.searchParams.set("code_challenge_method", "S256");
  url.searchParams.set("prompt", "none");
  return { url: url.toString(), state, verifier };
}

export function redirectUri(): string {
  return `${config.baseUrl}/api/auth/discord/callback`;
}

export async function exchangeCode(code: string, verifier: string): Promise<string> {
  const response = await fetch(`${API}/oauth2/token`, {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      client_id: config.discord.clientId ?? "",
      client_secret: config.discord.clientSecret ?? "",
      grant_type: "authorization_code",
      code,
      redirect_uri: redirectUri(),
      code_verifier: verifier,
    }),
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) throw new Error(`Discord token exchange answered ${response.status}`);
  const body = (await response.json()) as { access_token?: string };
  if (!body.access_token) throw new Error("Discord token exchange returned no access token");
  return body.access_token;
}

export async function fetchUser(accessToken: string): Promise<DiscordUser> {
  const response = await fetch(`${API}/users/@me`, {
    headers: { Authorization: `Bearer ${accessToken}` },
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) throw new Error(`Discord /users/@me answered ${response.status}`);
  return (await response.json()) as DiscordUser;
}

/** The member's role ids in the guild, or null when they are not in it. */
export async function fetchGuildRoles(
  accessToken: string,
  guildId: string,
): Promise<string[] | null> {
  const response = await fetch(`${API}/users/@me/guilds/${encodeURIComponent(guildId)}/member`, {
    headers: { Authorization: `Bearer ${accessToken}` },
    signal: AbortSignal.timeout(10_000),
  });
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`Discord guild member answered ${response.status}`);
  const body = (await response.json()) as { roles?: string[] };
  return body.roles ?? [];
}

/** The highest dashboard role any of the member's Discord roles maps to. */
export function roleFromGuild(
  memberRoles: string[],
  map: Record<Exclude<Role, "owner">, string[]>,
): Role | null {
  let best: Role | null = null;
  for (const [role, ids] of Object.entries(map) as [Exclude<Role, "owner">, string[]][]) {
    if (ids.some((id) => memberRoles.includes(id)) && (!best || rank(role) > rank(best)))
      best = role;
  }
  return best;
}

export function avatarUrl(discordId: string, avatar: string | null, size = 64): string | undefined {
  if (!avatar) return undefined;
  return `https://cdn.discordapp.com/avatars/${discordId}/${avatar}.webp?size=${size}`;
}
