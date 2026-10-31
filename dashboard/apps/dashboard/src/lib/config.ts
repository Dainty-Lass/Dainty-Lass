import { join, resolve } from "node:path";

/**
 * Environment, read once. Only what must be known before the database opens, plus secrets, lives here;
 * everything a person tunes is a setting (lib/settings.ts). Names carry the DASHBOARD_ prefix so the
 * host kit's `start`, which clears unknown variables, can be taught the one prefix.
 */
function env(name: string): string | undefined {
  const value = process.env[name]?.trim();
  return value ? value : undefined;
}

function list(name: string): string[] {
  return (env(name) ?? "")
    .split(",")
    .map((v) => v.trim())
    .filter(Boolean);
}

const dataDir = resolve(env("DASHBOARD_DATA_DIR") ?? join(process.cwd(), "data"));

export const config = {
  dataDir,
  databaseFile: env("DASHBOARD_DATABASE_FILE") ?? join(dataDir, "dashboard.db"),
  /** The public origin, for the OAuth redirect and the Secure cookie decision. */
  baseUrl: (env("DASHBOARD_BASE_URL") ?? "http://localhost:3000").replace(/\/+$/, ""),
  discord: {
    clientId: env("DASHBOARD_DISCORD_CLIENT_ID"),
    clientSecret: env("DASHBOARD_DISCORD_CLIENT_SECRET"),
  },
  /** Discord ids that become owners on first sign-in: how the first person gets in. */
  ownerDiscordIds: list("DASHBOARD_OWNER_DISCORD_IDS"),
  /** The admin service's GraphQL (docs/admin-api.md in dainty-lass): moderation and live status. */
  adminApi: {
    url: env("DASHBOARD_ADMIN_API_URL") ?? "http://127.0.0.1:61003/graphql",
    key: env("DASHBOARD_ADMIN_API_KEY"),
  },
  /** The backend's /admin/v1, signed as the manager operator (backend security/managerOperator.ts). */
  backend: {
    url: (env("DASHBOARD_BACKEND_URL") ?? "http://127.0.0.1:61000").replace(/\/+$/, ""),
    operatorSecret: env("DASHBOARD_MANAGER_OPERATOR_SECRET"),
  },
  /** What the backend sends with chat and reports; at least 32 characters or ingest stays off. */
  ingestKey: env("DASHBOARD_INGEST_KEY"),
  sessionDays: Number(env("DASHBOARD_SESSION_DAYS") || 7),
  /** Sign-in without Discord, for a dashboard on a laptop with no app registered. Never in production. */
  devLogin: env("DASHBOARD_DEV_LOGIN") === "on" && process.env.NODE_ENV !== "production",
} as const;

export function discordConfigured(): boolean {
  return Boolean(config.discord.clientId && config.discord.clientSecret);
}

export function secureCookies(): boolean {
  return config.baseUrl.toLowerCase().startsWith("https:");
}
