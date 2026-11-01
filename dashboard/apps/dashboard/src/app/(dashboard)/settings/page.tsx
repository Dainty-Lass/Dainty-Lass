import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { config, discordConfigured, twitchConfigured } from "@/lib/config";
import { listRevisions } from "@/lib/revisions";
import { getSettings } from "@/lib/settings";
import { SettingsClient } from "./SettingsClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("settings"))("title") };
}

export default async function SettingsPage() {
  const me = await requirePage("settings:read");
  return (
    <SettingsClient
      settings={getSettings()}
      canWrite={can(me.role, "settings:write")}
      canRestore={can(me.role, "audit:revert")}
      // Where things point, never the secrets themselves: only whether each is set.
      connections={{
        adminApiUrl: config.adminApi.url,
        adminApiKey: Boolean(config.adminApi.key),
        backendUrl: config.backend.url,
        operatorSecret: Boolean(config.backend.operatorSecret),
        ingestKey: Boolean(config.ingestKey && config.ingestKey.length >= 32),
        discord: discordConfigured(),
        redirectUris: {
          discord: `${config.baseUrl}/api/auth/callback/discord`,
          twitch: `${config.baseUrl}/api/auth/callback/twitch`,
        },
        twitch: twitchConfigured(),
        botToken: Boolean(config.discord.botToken),
        ownerCount: config.ownerDiscordIds.length + config.ownerEmails.length,
        dataDir: config.dataDir,
      }}
      revisions={listRevisions("settings", 20).map(({ snapshot, ...r }) => r)}
    />
  );
}
