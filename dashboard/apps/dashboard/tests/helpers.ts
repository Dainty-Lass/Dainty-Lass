import { and, eq } from "drizzle-orm";
import type { GuildStanding } from "@/lib/auth/discord";
import { db, schema } from "@/lib/db";
import type { Staff } from "@/lib/db/schema";
import { resolveSignIn, type SignInOutcome } from "@/lib/staff";

let n = 0;

/** A sign-in identity as Better Auth would have made it, with Discord linked when an id is given. */
export function identity(opts: { name: string; discordId?: string; email?: string }) {
  const now = new Date();
  // Test files share one process, and so one database: the same Discord account is the same person.
  const linked =
    opts.discordId &&
    db
      .select()
      .from(schema.authAccounts)
      .where(
        and(
          eq(schema.authAccounts.providerId, "discord"),
          eq(schema.authAccounts.accountId, opts.discordId),
        ),
      )
      .get();
  if (linked)
    return {
      userId: linked.userId,
      name: opts.name,
      image: null,
      email: opts.email ?? null,
      discordId: opts.discordId ?? null,
    };
  const user = db
    .insert(schema.authUsers)
    .values({
      name: opts.name,
      email: opts.email ?? `test-${++n}@test.invalid`,
      emailVerified: Boolean(opts.email),
      createdAt: now,
      updatedAt: now,
    })
    .returning()
    .get();
  if (opts.discordId)
    db.insert(schema.authAccounts)
      .values({
        userId: user.id,
        providerId: "discord",
        accountId: opts.discordId,
        createdAt: now,
        updatedAt: now,
      })
      .run();
  return {
    userId: user.id,
    name: opts.name,
    image: null,
    email: opts.email ?? null,
    discordId: opts.discordId ?? null,
  };
}

export function signIn(
  id: ReturnType<typeof identity>,
  guild?: GuildStanding | null,
): SignInOutcome {
  return resolveSignIn(id, guild);
}

export function signedIn(id: ReturnType<typeof identity>, guild?: GuildStanding | null): Staff {
  const outcome = signIn(id, guild);
  if (!outcome.ok) throw new Error(`sign-in refused: ${outcome.reason}`);
  return outcome.staff;
}
