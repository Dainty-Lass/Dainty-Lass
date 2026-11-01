import { randomBytes } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { passkey } from "@better-auth/passkey";
import { type BetterAuthPlugin, betterAuth } from "better-auth";
import { drizzleAdapter } from "better-auth/adapters/drizzle";
import { APIError, createAuthMiddleware, getSessionFromCtx } from "better-auth/api";
import { nextCookies } from "better-auth/next-js";
import { config, discordConfigured, twitchConfigured } from "@/lib/config";
import { db, schema } from "@/lib/db";
import { getSettings } from "@/lib/settings";
import { devSignIn } from "./dev-sign-in";
import {
  accountLinked,
  auditCredentialChange,
  refuseSignIn,
  signInMember,
  waysIn,
} from "./sign-in";

/** The configured secret, or one made once and kept beside the database so restarts keep sessions. */
function authSecret(): string {
  if (config.authSecret) return config.authSecret;
  const file = join(config.dataDir, "auth-secret");
  if (existsSync(file)) return readFileSync(file, "utf8").trim();
  mkdirSync(config.dataDir, { recursive: true });
  const secret = randomBytes(32).toString("base64url");
  writeFileSync(file, secret, { mode: 0o600 });
  return secret;
}

const base = new URL(config.baseUrl);

// guilds.members.read only when a guild is set: asking for more than we use reads as overreach. Read at
// start; changing the guild in Settings asks the new scope from the next restart.
const discordScopes = getSettings().discordGuildId
  ? ["identify", "email", "guilds.members.read"]
  : ["identify", "email"];

/**
 * Sign-in: Discord, Twitch, or a passkey added to an account. No passwords, no email sign-in.
 * Every session that starts goes through `signInMember`, which decides who the person is here.
 */
export const auth = betterAuth({
  appName: "Dainty Lass",
  baseURL: config.baseUrl,
  basePath: "/api/auth",
  secret: authSecret(),
  database: drizzleAdapter(db, { provider: "sqlite", schema }),
  advanced: {
    database: { generateId: "serial" },
    cookiePrefix: "dainty",
  },
  user: { modelName: "authUsers" },
  session: { modelName: "authSessions", expiresIn: config.sessionDays * 86_400 },
  verification: { modelName: "authVerifications" },
  account: {
    modelName: "authAccounts",
    encryptOAuthTokens: true,
    accountLinking: {
      enabled: true,
      // Both confirm the address they hand over, so the same address is the same person.
      trustedProviders: ["discord", "twitch"],
      // Linking from the Account page: a Twitch on another address is still yours once you are signed in.
      allowDifferentEmails: true,
    },
  },
  emailAndPassword: { enabled: false },
  socialProviders: {
    ...(discordConfigured()
      ? {
          discord: {
            clientId: config.discord.clientId as string,
            clientSecret: config.discord.clientSecret as string,
            scope: discordScopes,
            disableDefaultScope: true,
            prompt: "none" as const,
          },
        }
      : {}),
    ...(twitchConfigured()
      ? {
          twitch: {
            clientId: config.twitch.clientId as string,
            clientSecret: config.twitch.clientSecret as string,
          },
        }
      : {}),
  },
  plugins: [
    passkey({
      rpID: base.hostname,
      rpName: "Dainty Lass",
      origin: base.origin,
      // Discoverable and verified: the passkey alone names the account and stands for both factors.
      authenticatorSelection: { residentKey: "required", userVerification: "required" },
      schema: { passkey: { modelName: "authPasskeys" } },
    }) as unknown as BetterAuthPlugin,
    ...(config.devLogin ? [devSignIn()] : []),
    // Last: lets server actions that call auth.api set and clear the cookie through next/headers.
    nextCookies(),
  ],
  hooks: {
    // The last way in is never removed, or the account could not be entered again.
    before: createAuthMiddleware(async (ctx) => {
      if (ctx.path !== "/passkey/delete-passkey") return;
      const session = await getSessionFromCtx(ctx);
      if (session && waysIn(db, Number(session.user.id)) <= 1)
        throw new APIError("BAD_REQUEST", { message: "last_sign_in", code: "LAST_SIGN_IN" });
    }),
    after: createAuthMiddleware(async (ctx) => {
      // Refused requests change nothing, so they are not audited.
      if (ctx.context.returned instanceof Error) return;
      const userId = ctx.context.session?.user.id;
      if (userId) auditCredentialChange(Number(userId), ctx.path);
    }),
  },
  databaseHooks: {
    account: {
      create: {
        after: async (account) =>
          accountLinked(Number(account.userId), account.providerId, account.accountId),
      },
    },
    session: {
      create: {
        before: async (session, ctx) => {
          const outcome = await signInMember(Number(session.userId), ctx?.path ?? "unknown");
          if (!outcome.ok) {
            refuseSignIn(Number(session.userId), outcome.reason);
            throw new APIError("FORBIDDEN", {
              message: outcome.reason,
              code: outcome.reason.toUpperCase(),
            });
          }
        },
      },
    },
  },
});

export type Auth = typeof auth;
