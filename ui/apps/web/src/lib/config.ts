import { join, resolve } from "node:path";
import { settledUrl } from "@dainty/fleet";

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
  /** Signs session cookies and encrypts provider tokens; written to the data directory when unset. */
  authSecret: env("DASHBOARD_AUTH_SECRET"),
  discord: {
    clientId: env("DASHBOARD_DISCORD_CLIENT_ID"),
    clientSecret: env("DASHBOARD_DISCORD_CLIENT_SECRET"),
    // A bot in the guild with Manage Roles, so accepting the terms can add the approved role.
    botToken: env("DASHBOARD_DISCORD_BOT_TOKEN"),
  },
  twitch: {
    clientId: env("DASHBOARD_TWITCH_CLIENT_ID"),
    clientSecret: env("DASHBOARD_TWITCH_CLIENT_SECRET"),
  },
  /** Where sign-in codes are sent from: smtp(s)://user:pass@host:port. Without it, no email sign-in. */
  mail: {
    smtpUrl: env("DASHBOARD_SMTP_URL"),
    from: env("DASHBOARD_MAIL_FROM"),
  },
  /** Discord ids and verified addresses that become owners on sign-in: how the first person gets in. */
  ownerDiscordIds: list("DASHBOARD_OWNER_DISCORD_IDS"),
  ownerEmails: list("DASHBOARD_OWNER_EMAILS").map((e) => e.toLowerCase()),
  /** The admin service's GraphQL (docs/admin-api.md in dainty-lass): moderation and live status. */
  adminApi: {
    url: settledUrl(
      "DASHBOARD_ADMIN_API_URL",
      env("DASHBOARD_ADMIN_API_URL") ?? "http://127.0.0.1:41003/graphql",
    ),
    key: env("DASHBOARD_ADMIN_API_KEY"),
  },
  /** The backend's /admin/v1, signed as the manager operator (backend security/managerOperator.ts). */
  backend: {
    url: settledUrl(
      "DASHBOARD_BACKEND_URL",
      env("DASHBOARD_BACKEND_URL") ?? "http://127.0.0.1:41000",
    ).replace(/\/+$/, ""),
    operatorSecret: env("DASHBOARD_MANAGER_OPERATOR_SECRET"),
  },
  /** What the backend sends with chat and reports; at least 32 characters or ingest stays off. */
  ingestKey: env("DASHBOARD_INGEST_KEY"),
  sessionDays: Number(env("DASHBOARD_SESSION_DAYS") || 7),
  /** One-click sign-in per tier, for a dashboard on a laptop with no provider registered. Never in production. */
  devLogin: env("DASHBOARD_DEV_LOGIN") === "on" && process.env.NODE_ENV !== "production",
} as const;

export function discordConfigured(): boolean {
  return Boolean(config.discord.clientId && config.discord.clientSecret);
}

export function twitchConfigured(): boolean {
  return Boolean(config.twitch.clientId && config.twitch.clientSecret);
}

/** Email codes need somewhere to send them; a dev box without SMTP prints them instead. */
export function emailSignInAvailable(): boolean {
  return Boolean(config.mail.smtpUrl && config.mail.from) || config.devLogin;
}

export function secureCookies(): boolean {
  return config.baseUrl.toLowerCase().startsWith("https:");
}
