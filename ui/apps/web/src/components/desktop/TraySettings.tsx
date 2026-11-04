"use client";

import { Stack } from "@astryxdesign/core/Stack";
import { Switch } from "@astryxdesign/core/Switch";
import type { Settings } from "@dainty/bridge";
import { useTranslations } from "next-intl";

/** Both shells' minimise- and close-to-tray switches; the shell does the hiding (tray.rs). */
export function TraySettings({
  settings,
  set,
}: {
  settings: Settings;
  set: (patch: Partial<Settings>) => void;
}) {
  const t = useTranslations("tray");
  return (
    <Stack gap={4}>
      <Switch
        label={t("minimize")}
        description={t("minimizeHint")}
        value={settings.minimizeToTray}
        onChange={(v) => set({ minimizeToTray: v })}
      />
      <Switch
        label={t("close")}
        description={t("closeHint")}
        value={settings.closeToTray}
        onChange={(v) => set({ closeToTray: v })}
      />
    </Stack>
  );
}
