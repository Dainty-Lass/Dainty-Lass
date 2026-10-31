import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Center } from "@astryxdesign/core/Center";
import { Heading } from "@astryxdesign/core/Heading";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { BrandMark } from "@dainty/theme";
import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { getTranslations } from "next-intl/server";
import { DiscordIcon } from "@/components/DiscordIcon";
import { currentStaff } from "@/lib/auth/session";
import { config, discordConfigured } from "@/lib/config";
import { getSettings } from "@/lib/settings";

export async function generateMetadata(): Promise<Metadata> {
  const t = await getTranslations("login");
  return { title: t("title") };
}

const ERRORS = [
  "not_staff",
  "disabled",
  "discord_denied",
  "discord_failed",
  "state_mismatch",
  "discord_not_configured",
] as const;

export default async function LoginPage({
  searchParams,
}: {
  searchParams: Promise<{ error?: string }>;
}) {
  if (await currentStaff()) redirect("/");
  const [{ error }, t] = await Promise.all([searchParams, getTranslations("login")]);
  const { serverName } = getSettings();
  const known = ERRORS.find((e) => e === error);
  return (
    <Center height="100dvh">
      <Stack gap={4} width={400} maxWidth="calc(100vw - 32px)">
        <Card padding={6} elevation="low">
          <Stack gap={5}>
            <Stack gap={3} align="center">
              <BrandMark size={56} />
              <Stack gap={1} align="center">
                <Heading level={1} justify="center">
                  {serverName}
                </Heading>
                <Text type="supporting" justify="center">
                  {t("subtitle")}
                </Text>
              </Stack>
            </Stack>
            {known && <Banner status="error" title={t(`errors.${known}`)} collapsible={false} />}
            <Button
              label={t("discord")}
              variant="primary"
              size="lg"
              width="100%"
              href="/api/auth/discord/start"
              icon={<DiscordIcon />}
              isDisabled={!discordConfigured()}
              tooltip={discordConfigured() ? undefined : t("errors.discord_not_configured")}
            />
            {config.devLogin && (
              <form method="post" action="/api/auth/dev">
                <Button label={t("dev")} type="submit" variant="ghost" width="100%" />
              </form>
            )}
          </Stack>
        </Card>
        <Text type="supporting" justify="center">
          {t("staffOnly")}
        </Text>
      </Stack>
    </Center>
  );
}
