import { eq } from "drizzle-orm";
import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { requirePage } from "@/lib/auth/guard";
import { discordConfigured, emailSignInAvailable, twitchConfigured } from "@/lib/config";
import { db, schema } from "@/lib/db";
import { AccountClient } from "./AccountClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("account"))("title") };
}

/** Everyone's own page: how they sign in, and the passkeys on their account. */
export default async function AccountPage({
  searchParams,
}: {
  searchParams: Promise<{ error?: string }>;
}) {
  const me = await requirePage("overview:read");
  const { error } = await searchParams;
  const user = db.select().from(schema.authUsers).where(eq(schema.authUsers.id, me.userId)).get();
  const accounts = db
    .select({
      id: schema.authAccounts.id,
      providerId: schema.authAccounts.providerId,
      accountId: schema.authAccounts.accountId,
      createdAt: schema.authAccounts.createdAt,
    })
    .from(schema.authAccounts)
    .where(eq(schema.authAccounts.userId, me.userId))
    .all();
  const passkeys = db
    .select({
      id: schema.authPasskeys.id,
      name: schema.authPasskeys.name,
      deviceType: schema.authPasskeys.deviceType,
      backedUp: schema.authPasskeys.backedUp,
      createdAt: schema.authPasskeys.createdAt,
    })
    .from(schema.authPasskeys)
    .where(eq(schema.authPasskeys.userId, me.userId))
    .all();
  // The placeholder a Discord-only account gets is not an address anyone can receive a code at.
  const email = user && !user.email.endsWith(".invalid") ? user.email : null;
  return (
    <AccountClient
      email={email}
      emailVerified={Boolean(user?.emailVerified)}
      accounts={accounts.map((a) => ({ ...a, createdAt: a.createdAt.getTime() }))}
      passkeys={passkeys.map((p) => ({
        ...p,
        id: String(p.id),
        createdAt: p.createdAt?.getTime() ?? null,
      }))}
      providers={{
        discord: discordConfigured(),
        twitch: twitchConfigured(),
        email: emailSignInAvailable(),
      }}
      linkError={error ?? null}
    />
  );
}
