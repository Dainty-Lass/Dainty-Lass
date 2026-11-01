import { Card } from "@astryxdesign/core/Card";
import { Center } from "@astryxdesign/core/Center";
import { Heading } from "@astryxdesign/core/Heading";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { BrandMark } from "@dainty/theme";
import type { Metadata } from "next";
import { redirect } from "next/navigation";
import { getTranslations } from "next-intl/server";
import { currentStaff } from "@/lib/auth/session";
import { config, discordConfigured, emailSignInAvailable, twitchConfigured } from "@/lib/config";
import { getSettings } from "@/lib/settings";
import { LoginClient } from "./LoginClient";

export async function generateMetadata(): Promise<Metadata> {
  const t = await getTranslations("login");
  return { title: t("title") };
}

export default async function LoginPage({
  searchParams,
}: {
  searchParams: Promise<{ error?: string }>;
}) {
  if (await currentStaff()) redirect("/");
  const [{ error }, t] = await Promise.all([searchParams, getTranslations("login")]);
  const { serverName } = getSettings();
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
            <LoginClient
              providers={{
                discord: discordConfigured(),
                twitch: twitchConfigured(),
                email: emailSignInAvailable(),
              }}
              dev={config.devLogin}
              initialError={error ? error.toLowerCase() : null}
            />
          </Stack>
        </Card>
        <Text type="supporting" justify="center">
          {t("anyone")}
        </Text>
      </Stack>
    </Center>
  );
}
