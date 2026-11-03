"use client";

import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Heading } from "@astryxdesign/core/Heading";
import { MetadataList, MetadataListItem } from "@astryxdesign/core/MetadataList";
import { Selector } from "@astryxdesign/core/Selector";
import { Stack } from "@astryxdesign/core/Stack";
import { Switch } from "@astryxdesign/core/Switch";
import { Text } from "@astryxdesign/core/Text";
import type { GameLanguage, GraphicsPreset, Language, Settings } from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { ThemeModeControl } from "@dainty/theme";
import { FolderOpen } from "lucide-react";
import { useTranslations } from "next-intl";
import type { ReactNode } from "react";
import { LauncherShell } from "@/components/LauncherShell";

const GRAPHICS: GraphicsPreset[] = [-1, 0, 1, 2, 3, 4];
const GAME_LANGUAGES: GameLanguage[] = [
  "auto",
  "en-US",
  "de-DE",
  "es-ES",
  "fr-FR",
  "it-IT",
  "ja-JP",
  "pt-BR",
  "ru-RU",
];

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <Card padding={5}>
      <Stack gap={4}>
        <Heading level={2}>{title}</Heading>
        {children}
      </Stack>
    </Card>
  );
}

export default function SettingsPage() {
  const t = useTranslations("settings");
  const tc = useTranslations("theme");
  const { snapshot, api } = useBridge();
  if (!snapshot || !api) return null;
  const s = snapshot.settings;
  const set = (patch: Partial<Settings>) => void api.setSettings(patch);

  return (
    <LauncherShell>
      <Stack gap={4} padding={6} isScrollable height="100%" maxWidth={820}>
        <Heading level={1}>{t("title")}</Heading>

        <Section title={t("game")}>
          <Selector
            label={t("graphics")}
            value={String(s.graphics)}
            onChange={(v) => set({ graphics: Number(v) as GraphicsPreset })}
            options={GRAPHICS.map((g) => ({ value: String(g), label: t(`graphicsPresets.${g}`) }))}
            width={320}
          />
          <Switch label={t("windowed")} value={s.windowed} onChange={(v) => set({ windowed: v })} />
          <Selector
            label={t("gameLanguage")}
            value={s.gameLanguage}
            onChange={(v) => set({ gameLanguage: v as GameLanguage })}
            options={GAME_LANGUAGES.map((l) => ({ value: l, label: t(`gameLanguages.${l}`) }))}
            width={320}
          />
          <Stack direction="horizontal" gap={2}>
            <Button
              label={t("openGameFolder")}
              icon={<FolderOpen size={16} />}
              onClick={() => void api.openGameFolder()}
              isDisabled={!snapshot.game.root}
            />
            <Button
              label={t("verify")}
              clickAction={async () => void (await api.verifyGame())}
              isDisabled={!snapshot.game.root || snapshot.busy}
            />
          </Stack>
        </Section>

        <Section title={t("launcher")}>
          <Selector
            label={t("language")}
            value={s.language}
            onChange={(v) => set({ language: v as Language })}
            options={[
              { value: "en", label: "English" },
              { value: "pt-BR", label: "Português (Brasil)" },
            ]}
            width={320}
          />
          <ThemeModeControl
            label={tc("label")}
            labels={{ system: tc("system"), light: tc("light"), dark: tc("dark") }}
            size="md"
          />
          <Switch
            label={t("streamerMode")}
            description={t("streamerModeHint")}
            value={s.streamerMode}
            onChange={(v) => set({ streamerMode: v })}
          />
          {snapshot.app.hostToolsAvailable && (
            <Switch
              label={t("hostTools")}
              description={t("hostToolsHint")}
              value={s.hostTools}
              onChange={(v) => set({ hostTools: v })}
            />
          )}
          <Button
            label={t("openLogs")}
            icon={<FolderOpen size={16} />}
            onClick={() => void api.openLogsFolder()}
          />
        </Section>

        <Section title={t("account")}>
          <MetadataList label={{ position: "start", width: 160 }}>
            <MetadataListItem label={t("username")}>
              {s.streamerMode ? "•••" : (snapshot.account.username ?? "—")}
            </MetadataListItem>
            <MetadataListItem label={t("server")}>{snapshot.server?.name ?? "—"}</MetadataListItem>
            <MetadataListItem label={t("version")}>
              <Text type="code">{snapshot.app.version}</Text>
            </MetadataListItem>
          </MetadataList>
          <Stack direction="horizontal" gap={2}>
            <Button
              label={t("backupKey")}
              clickAction={async () => void (await api.saveKeyBackup())}
              isDisabled={!snapshot.account.hasKey}
            />
            <Button
              label={t("signOut")}
              variant="destructive"
              clickAction={async () => void (await api.logout())}
              isDisabled={!snapshot.account.hasKey}
            />
          </Stack>
          <Text type="supporting">{t("signOutHint")}</Text>
        </Section>
      </Stack>
    </LauncherShell>
  );
}
