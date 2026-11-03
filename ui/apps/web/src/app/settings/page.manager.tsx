"use client";

import { Card } from "@astryxdesign/core/Card";
import { Heading } from "@astryxdesign/core/Heading";
import { Stack } from "@astryxdesign/core/Stack";
import type { Settings } from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { useTranslations } from "next-intl";
import { TraySettings } from "@/components/desktop/TraySettings";
import { ManagerShell } from "@/components/manager/ManagerShell";

export default function SettingsPage() {
  const t = useTranslations("manager.settings");
  const tt = useTranslations("tray");
  const { snapshot, api } = useBridge();
  if (!snapshot || !api) return null;
  const set = (patch: Partial<Settings>) => void api.setSettings(patch);

  return (
    <ManagerShell title={t("title")}>
      <Card padding={5}>
        <Stack gap={4}>
          <Heading level={2}>{tt("title")}</Heading>
          <TraySettings settings={snapshot.settings} set={set} />
        </Stack>
      </Card>
    </ManagerShell>
  );
}
