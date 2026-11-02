import { config } from "@/lib/config";
import { type Role, rank } from "./permissions";

const API = "https://discord.com/api/v10";

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

/** What the guild says about someone at sign-in: the tier its roles grant, and whether they are approved. */
export interface GuildStanding {
  role: Role | null;
  approved: boolean;
}

export function standingFromGuild(
  memberRoles: string[] | null,
  map: Record<Exclude<Role, "owner">, string[]>,
  approvedRoleId: string,
): GuildStanding | null {
  if (!memberRoles) return null;
  return {
    role: roleFromGuild(memberRoles, map),
    approved: Boolean(approvedRoleId) && memberRoles.includes(approvedRoleId),
  };
}

/**
 * Gives a member a role with the dashboard's bot (Manage Roles, and ranked above the role it adds).
 * Answers whether Discord accepted; the caller records approval either way.
 */
export async function addGuildRole(
  guildId: string,
  userId: string,
  roleId: string,
): Promise<boolean> {
  if (!config.discord.botToken) return false;
  const response = await fetch(
    `${API}/guilds/${encodeURIComponent(guildId)}/members/${encodeURIComponent(userId)}/roles/${encodeURIComponent(roleId)}`,
    {
      method: "PUT",
      headers: {
        Authorization: `Bot ${config.discord.botToken}`,
        "X-Audit-Log-Reason": "Accepted the server terms in the DaintyLass dashboard",
      },
      signal: AbortSignal.timeout(10_000),
    },
  ).catch(() => null);
  return response?.status === 204;
}
