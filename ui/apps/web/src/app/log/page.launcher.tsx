"use client";

import { Button } from "@astryxdesign/core/Button";
import { Heading } from "@astryxdesign/core/Heading";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Text } from "@astryxdesign/core/Text";
import { useBridge } from "@dainty/bridge/react";
import { Copy } from "lucide-react";
import { useTranslations } from "next-intl";
import { toast } from "sonner";
import { ConsoleView } from "@/components/desktop/ConsoleView";
import { LauncherShell } from "@/components/launcher/LauncherShell";

/** copyText's limit (bridge spec COPY_TEXT_MAX): the newest lines that fit. */
const COPY_MAX = 4096;

function newestThatFit(lines: string[]): string {
  const kept: string[] = [];
  let size = 0;
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = lines[i] ?? "";
    if (size + line.length + 1 > COPY_MAX) break;
    kept.unshift(line);
    size += line.length + 1;
  }
  return kept.join("\n");
}

/** What the game's console shows, read by the launcher (platform/game_console.rs), the account key masked. */
export default function GameLogPage() {
  const t = useTranslations("launcher.gameLog");
  const { snapshot, api, gameLog } = useBridge();
  const running = snapshot?.running.game ?? false;

  return (
    <LauncherShell>
      <Stack gap={4} padding={6} height="100%">
        <Stack direction="horizontal" gap={4} justify="between" align="end" wrap="wrap">
          <Stack gap={1}>
            <Heading level={1}>{t("title")}</Heading>
            <Text type="supporting">{t("description")}</Text>
          </Stack>
          <Stack direction="horizontal" gap={3} align="center">
            <Stack direction="horizontal" gap={2} align="center">
              <StatusDot
                variant={running ? "success" : "neutral"}
                label={running ? t("running") : t("stopped")}
              />
              <Text type="supporting">{running ? t("running") : t("stopped")}</Text>
            </Stack>
            <Button
              label={t("copy")}
              icon={<Copy size={16} />}
              isDisabled={!api || gameLog.length === 0}
              clickAction={async () => {
                if (api && (await api.copyText(newestThatFit(gameLog)))) toast.success(t("copied"));
              }}
            />
          </Stack>
        </Stack>
        <ConsoleView lines={gameLog} emptyText={t("empty")} fill />
      </Stack>
    </LauncherShell>
  );
}
