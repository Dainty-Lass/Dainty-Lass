import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { getTranslations } from "next-intl/server";
import { requireSignedIn } from "@/lib/auth/guard";
import { needsOnboarding } from "@/lib/onboarding";
import { getSettings } from "@/lib/settings";
import { WelcomeClient } from "./WelcomeClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("welcome"))("title") };
}

export default async function WelcomePage() {
  const me = await requireSignedIn();
  const s = getSettings();
  // Already approved, or the server does not ask: nothing to do here.
  if (!needsOnboarding(me, s)) redirect("/");
  return (
    <WelcomeClient
      serverName={s.serverName}
      me={{
        name: me.displayName || me.globalName || me.username,
        avatar: me.avatar ?? undefined,
        gameName: me.gameName ?? "",
        renewing: me.approvedVia === "tos" && Boolean(me.approvedAt),
      }}
      tos={{ text: s.tosText, version: s.tosVersion }}
      addsRole={Boolean(s.discordGuildId && s.discordApprovedRoleId)}
    />
  );
}
