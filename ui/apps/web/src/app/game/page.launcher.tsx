"use client";

import { Center } from "@astryxdesign/core/Center";
import { Heading } from "@astryxdesign/core/Heading";
import { Spinner } from "@astryxdesign/core/Spinner";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { useBridge } from "@dainty/bridge/react";
import { useTranslations } from "next-intl";
import { EpicInstall } from "@/components/launcher/EpicInstall";
import { LauncherShell } from "@/components/launcher/LauncherShell";

/** The game download from the player's own Epic account (DE7), reachable before any server is joined. */
export default function GamePage() {
  const t = useTranslations("launcher.install");
  const { snapshot } = useBridge();

  if (!snapshot) {
    return (
      <Center height="100dvh">
        <Spinner size="lg" />
      </Center>
    );
  }

  return (
    <LauncherShell>
      <Stack gap={4} padding={6} isScrollable height="100%">
        <Stack gap={1}>
          <Heading level={1}>{t("title")}</Heading>
          <Text type="supporting">{t("description")}</Text>
        </Stack>
        <EpicInstall snapshot={snapshot} />
      </Stack>
    </LauncherShell>
  );
}
