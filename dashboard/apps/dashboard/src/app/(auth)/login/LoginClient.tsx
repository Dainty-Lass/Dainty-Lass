"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { KeyRound } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { DiscordIcon } from "@/components/DiscordIcon";
import { TwitchIcon } from "@/components/TwitchIcon";
import { authClient } from "@/lib/auth/client";

const DEV_TIERS = ["owner", "admin", "moderator", "user", "newcomer"] as const;
const ERRORS = [
  "disabled",
  "discord_failed",
  "twitch_failed",
  "passkey_failed",
  "dev_failed",
  "unknown",
] as const;
type LoginError = (typeof ERRORS)[number];

/** Every way in: Discord, Twitch or a passkey. There are no passwords. */
export function LoginClient({
  providers,
  dev,
  initialError,
}: {
  providers: { discord: boolean; twitch: boolean };
  dev: boolean;
  initialError: string | null;
}) {
  const t = useTranslations("login");
  // Better Auth sends people back with its own code; anything we do not name reads as "try again".
  const [error, setError] = useState<LoginError | null>(
    initialError ? (ERRORS.find((e) => e === initialError) ?? "unknown") : null,
  );
  const [busy, setBusy] = useState<string | null>(null);

  const fail = (e: LoginError) => {
    setBusy(null);
    setError(e);
  };
  const land = () => window.location.assign("/");

  async function social(provider: "discord" | "twitch") {
    setBusy(provider);
    const r = await authClient.signIn.social({
      provider,
      callbackURL: "/",
      errorCallbackURL: "/login",
    });
    if (r.error) fail(`${provider}_failed`);
  }

  async function passkey() {
    setBusy("passkey");
    const r = await authClient.signIn.passkey();
    if (r?.error) fail(r.error.status === 403 ? "disabled" : "passkey_failed");
    else land();
  }

  async function devAs(as: (typeof DEV_TIERS)[number]) {
    setBusy(`dev-${as}`);
    const r = await fetch("/api/auth/dev/sign-in", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ as }),
    });
    if (!r.ok) fail(r.status === 403 ? "disabled" : "dev_failed");
    else land();
  }

  return (
    <Stack gap={4}>
      {error && <Banner status="error" title={t(`errors.${error}`)} collapsible={false} />}
      <Stack gap={2}>
        {providers.discord && (
          <Button
            label={t("discord")}
            variant="primary"
            size="lg"
            width="100%"
            icon={<DiscordIcon />}
            isLoading={busy === "discord"}
            onClick={() => social("discord")}
          />
        )}
        {providers.twitch && (
          <Button
            label={t("twitch")}
            variant="secondary"
            size="lg"
            width="100%"
            icon={<TwitchIcon />}
            isLoading={busy === "twitch"}
            onClick={() => social("twitch")}
          />
        )}
        <Button
          label={t("passkey")}
          variant="secondary"
          size="lg"
          width="100%"
          icon={<KeyRound />}
          isLoading={busy === "passkey"}
          onClick={passkey}
        />
      </Stack>

      {dev && (
        <Stack gap={1}>
          <Text type="supporting" justify="center">
            {t("dev")}
          </Text>
          <Stack direction="horizontal" gap={1} justify="center" wrap="wrap">
            {DEV_TIERS.map((as) => (
              <Button
                key={as}
                label={t(`devAs.${as}`)}
                variant="ghost"
                size="sm"
                isLoading={busy === `dev-${as}`}
                onClick={() => devAs(as)}
              />
            ))}
          </Stack>
        </Stack>
      )}
    </Stack>
  );
}
