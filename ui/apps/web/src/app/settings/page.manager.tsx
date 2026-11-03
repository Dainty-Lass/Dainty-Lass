"use client";

import { Card } from "@astryxdesign/core/Card";
import { Heading } from "@astryxdesign/core/Heading";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import type { Settings } from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { useTranslations } from "next-intl";
import { AppearanceSettings } from "@/components/AppearanceSettings";
import { TraySettings } from "@/components/desktop/TraySettings";
import { ManagerShell } from "@/components/manager/ManagerShell";

export default function SettingsPage() {
  const t = useTranslations("manager.settings");
  const tt = useTranslations("tray");
  const ta = useTranslations("appearance");
  const { snapshot, api } = useBridge();
  if (!snapshot || !api) return null;
  const set = (patch: Partial<Settings>) => void api.setSettings(patch);

  return (
    <ManagerShell title={t("title")}>
      <Card padding={5}>
        <Stack gap={4}>
          <Stack gap={1}>
            <Heading level={2}>{ta("title")}</Heading>
            <Text type="supporting">{ta("hint")}</Text>
          </Stack>
          <AppearanceSettings />
        </Stack>
      </Card>
      <Card padding={5}>
        <Stack gap={4}>
          <Heading level={2}>{tt("title")}</Heading>
          <TraySettings settings={snapshot.settings} set={set} />
        </Stack>
      </Card>
    </ManagerShell>
  );
}
