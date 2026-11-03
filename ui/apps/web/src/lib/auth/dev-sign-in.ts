import type { BetterAuthPlugin } from "better-auth";
import { APIError, createAuthEndpoint } from "better-auth/api";
import { setSessionCookie } from "better-auth/cookies";
import { eq } from "drizzle-orm";
import { config } from "@/lib/config";
import { db, schema } from "@/lib/db";

// One made-up account per tier, so each view can be tried without registering a provider.
export const DEV_TIERS = ["owner", "admin", "moderator", "user", "newcomer"] as const;
export type DevTier = (typeof DEV_TIERS)[number];

/**
 * POST /api/auth/dev/sign-in {as}: only registered when DASHBOARD_DEV_LOGIN=on outside production. The
 * session hook makes the member as for anyone; the tier is then set by hand, and newcomer is left a plain
 * user nobody has approved.
 */
export function devSignIn(): BetterAuthPlugin {
  return {
    id: "dainty-dev-sign-in",
    endpoints: {
      devSignIn: createAuthEndpoint("/dev/sign-in", { method: "POST" }, async (ctx) => {
        if (!config.devLogin) throw new APIError("NOT_FOUND");
        const as = String((ctx.body as { as?: unknown } | undefined)?.as ?? "");
        if (!DEV_TIERS.includes(as as DevTier)) throw new APIError("BAD_REQUEST");
        const email = `dev-${as}@dev.invalid`;
        const found = await ctx.context.internalAdapter.findUserByEmail(email);
        const user =
          found?.user ??
          (await ctx.context.internalAdapter.createUser(
            { email, name: `Dev ${as}`, emailVerified: true },
            ctx,
          ));
        const session = await ctx.context.internalAdapter.createSession(user.id);
        const member = db
          .select()
          .from(schema.staff)
          .where(eq(schema.staff.userId, Number(user.id)))
          .get();
        if (member && as !== "newcomer") {
          db.update(schema.staff)
            .set({
              role: as as "admin",
              roleSource: "manual",
              approvedAt: member.approvedAt ?? Date.now(),
              approvedVia: member.approvedVia ?? "manual",
            })
            .where(eq(schema.staff.id, member.id))
            .run();
        }
        await setSessionCookie(ctx, { session, user });
        return ctx.json({ ok: true });
      }),
    },
  } satisfies BetterAuthPlugin;
}
