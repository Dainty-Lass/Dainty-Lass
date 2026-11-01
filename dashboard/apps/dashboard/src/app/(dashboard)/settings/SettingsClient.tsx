"use client";

import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Heading } from "@astryxdesign/core/Heading";
import { MetadataList, MetadataListItem } from "@astryxdesign/core/MetadataList";
import { NumberInput } from "@astryxdesign/core/NumberInput";
import { SegmentedControl, SegmentedControlItem } from "@astryxdesign/core/SegmentedControl";
import { Stack } from "@astryxdesign/core/Stack";
import { Switch } from "@astryxdesign/core/Switch";
import { Text } from "@astryxdesign/core/Text";
import { TextArea } from "@astryxdesign/core/TextArea";
import { TextInput } from "@astryxdesign/core/TextInput";
import { ACCENTS } from "@dainty/theme/mode";
import { useTranslations } from "next-intl";
import { type ReactNode, useState } from "react";
import { toast } from "sonner";
import { Page } from "@/components/Page";
import { RevisionList, type RevisionRow } from "@/components/RevisionList";
import { useAction } from "@/components/useAction";
import type { Settings } from "@/lib/settings";
import { pruneChatAction, saveSettingsAction } from "./actions";

const ids = (list: string[]) => list.join(", ");
const parseIds = (text: string) => text.split(/[\s,]+/).filter(Boolean);

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <Card padding={5}>
      <Stack gap={4}>
        <Stack gap={1}>
          <Heading level={2}>{title}</Heading>
          {description && <Text type="supporting">{description}</Text>}
        </Stack>
        {children}
      </Stack>
    </Card>
  );
}

export function SettingsClient({
  settings,
  canWrite,
  canRestore,
  connections,
  revisions,
}: {
  settings: Settings;
  canWrite: boolean;
  canRestore: boolean;
  connections: {
    adminApiUrl: string;
    adminApiKey: boolean;
    backendUrl: string;
    operatorSecret: boolean;
    ingestKey: boolean;
    discord: boolean;
    twitch: boolean;
    email: boolean;
    mail: boolean;
    botToken: boolean;
    redirectUris: { discord: string; twitch: string };
    ownerCount: number;
    dataDir: string;
  };
  revisions: RevisionRow[];
}) {
  const t = useTranslations("settings");
  const tc = useTranslations("common");
  const { run } = useAction();
  const [draft, setDraft] = useState(settings);
  const [roleText, setRoleText] = useState({
    admin: ids(settings.discordRoleMap.admin),
    moderator: ids(settings.discordRoleMap.moderator),
    user: ids(settings.discordRoleMap.user),
  });
  const [reasons, setReasons] = useState(settings.moderationReasons.join("\n"));

  const save = (patch: Partial<Settings>) => run(() => saveSettingsAction(patch), t("saved"));
  const set = <K extends keyof Settings>(key: K, value: Settings[K]) =>
    setDraft((d) => ({ ...d, [key]: value }));
  const ro = !canWrite;
  const yes = (on: boolean) => (
    <Badge variant={on ? "success" : "neutral"} label={on ? t("set") : t("unset")} />
  );

  return (
    <Page title={t("title")} description={canWrite ? t("description") : t("readOnly")} width={960}>
      <Section title={t("general")}>
        <TextInput
          label={t("serverName")}
          description={t("serverNameHint")}
          value={draft.serverName}
          onChange={(v) => set("serverName", v)}
          isReadOnly={ro}
        />
        <SegmentedControl
          label={t("accent")}
          value={draft.accent}
          onChange={(v) => set("accent", v as Settings["accent"])}
          isDisabled={ro}
        >
          {ACCENTS.map((a) => (
            <SegmentedControlItem key={a} value={a} label={t(`accents.${a}`)} />
          ))}
        </SegmentedControl>
        {canWrite && (
          <Stack direction="horizontal" justify="end">
            <Button
              label={tc("save")}
              variant="primary"
              clickAction={async () =>
                void (await save({ serverName: draft.serverName, accent: draft.accent }))
              }
            />
          </Stack>
        )}
      </Section>

      <Section title={t("discord")} description={t("discordHint")}>
        <TextInput
          label={t("guildId")}
          description={t("guildIdHint")}
          value={draft.discordGuildId}
          onChange={(v) => set("discordGuildId", v.trim())}
          isReadOnly={ro}
        />
        {(["admin", "moderator", "user"] as const).map((r) => (
          <TextInput
            key={r}
            label={t("roleIds", { role: tc(`roles.${r}`) })}
            value={roleText[r]}
            onChange={(v) => setRoleText((x) => ({ ...x, [r]: v }))}
            isReadOnly={ro || !draft.discordGuildId}
            placeholder={t("roleIdsPlaceholder")}
          />
        ))}
        <TextInput
          label={t("approvedRoleId")}
          description={t("approvedRoleIdHint")}
          value={draft.discordApprovedRoleId}
          onChange={(v) => set("discordApprovedRoleId", v.trim())}
          isReadOnly={ro || !draft.discordGuildId}
        />
        {canWrite && (
          <Stack direction="horizontal" justify="end">
            <Button
              label={tc("save")}
              variant="primary"
              clickAction={async () =>
                void (await save({
                  discordGuildId: draft.discordGuildId,
                  discordRoleMap: {
                    admin: parseIds(roleText.admin),
                    moderator: parseIds(roleText.moderator),
                    user: parseIds(roleText.user),
                  },
                  discordApprovedRoleId: draft.discordApprovedRoleId,
                }))
              }
            />
          </Stack>
        )}
      </Section>

      <Section title={t("onboarding")} description={t("onboardingHint")}>
        <Switch
          label={t("requireApproval")}
          description={t("requireApprovalHint")}
          value={draft.requireApproval}
          onChange={(v) => set("requireApproval", v)}
          isDisabled={ro}
        />
        <TextArea
          label={t("tos")}
          description={t("tosHint", { version: settings.tosVersion })}
          value={draft.tosText}
          onChange={(v) => set("tosText", v)}
          rows={10}
          isReadOnly={ro}
        />
        {draft.requireApproval && draft.discordApprovedRoleId && !connections.botToken && (
          <Banner status="warning" title={t("botMissing")} collapsible={false} />
        )}
        {canWrite && (
          <Stack direction="horizontal" justify="end">
            <Button
              label={tc("save")}
              variant="primary"
              clickAction={async () =>
                void (await save({
                  requireApproval: draft.requireApproval,
                  tosText: draft.tosText,
                }))
              }
            />
          </Stack>
        )}
      </Section>

      <Section title={t("moderation")}>
        <TextArea
          label={t("reasons")}
          description={t("reasonsHint")}
          value={reasons}
          onChange={setReasons}
          rows={6}
          isReadOnly={ro}
        />
        <NumberInput
          label={t("contextMinutes")}
          description={t("contextMinutesHint")}
          value={draft.reportContextMinutes}
          onChange={(v) => set("reportContextMinutes", v)}
          isReadOnly={ro}
        />
        {canWrite && (
          <Stack direction="horizontal" justify="end">
            <Button
              label={tc("save")}
              variant="primary"
              clickAction={async () =>
                void (await save({
                  moderationReasons: reasons.split("\n"),
                  reportContextMinutes: draft.reportContextMinutes,
                }))
              }
            />
          </Stack>
        )}
      </Section>

      <Section title={t("retention")} description={t("retentionHint")}>
        <NumberInput
          label={t("retentionDays")}
          description={t("retentionDaysHint")}
          value={draft.chatRetentionDays}
          onChange={(v) => set("chatRetentionDays", v)}
          isReadOnly={ro}
        />
        {canWrite && (
          <Stack direction="horizontal" gap={2} justify="end">
            <Button
              label={t("pruneNow")}
              clickAction={async () => {
                const removed = await run(() => pruneChatAction());
                if (removed !== false) toast.success(t("pruned", { count: Number(removed) }));
              }}
            />
            <Button
              label={tc("save")}
              variant="primary"
              clickAction={async () =>
                void (await save({ chatRetentionDays: draft.chatRetentionDays }))
              }
            />
          </Stack>
        )}
      </Section>

      <Section title={t("connections")} description={t("connectionsHint")}>
        <MetadataList label={{ position: "start", width: 220 }}>
          <MetadataListItem label={t("conn.adminApiUrl")}>
            <Text type="code">{connections.adminApiUrl}</Text>
          </MetadataListItem>
          <MetadataListItem label="DASHBOARD_ADMIN_API_KEY">
            {yes(connections.adminApiKey)}
          </MetadataListItem>
          <MetadataListItem label={t("conn.backendUrl")}>
            <Text type="code">{connections.backendUrl}</Text>
          </MetadataListItem>
          <MetadataListItem label="DASHBOARD_MANAGER_OPERATOR_SECRET">
            {yes(connections.operatorSecret)}
          </MetadataListItem>
          <MetadataListItem label="DASHBOARD_INGEST_KEY">
            {yes(connections.ingestKey)}
          </MetadataListItem>
          <MetadataListItem label={t("conn.discordApp")}>
            {yes(connections.discord)}
          </MetadataListItem>
          <MetadataListItem label={t("conn.redirect", { provider: "Discord" })}>
            <Text type="code">{connections.redirectUris.discord}</Text>
          </MetadataListItem>
          <MetadataListItem label="DASHBOARD_DISCORD_BOT_TOKEN">
            {yes(connections.botToken)}
          </MetadataListItem>
          <MetadataListItem label={t("conn.twitchApp")}>{yes(connections.twitch)}</MetadataListItem>
          <MetadataListItem label={t("conn.redirect", { provider: "Twitch" })}>
            <Text type="code">{connections.redirectUris.twitch}</Text>
          </MetadataListItem>
          <MetadataListItem label="DASHBOARD_SMTP_URL">{yes(connections.mail)}</MetadataListItem>
          <MetadataListItem label={t("conn.emailSignIn")}>
            {yes(connections.email)}
          </MetadataListItem>
          <MetadataListItem label={t("conn.ownersLabel")}>
            {t("conn.owners", { count: connections.ownerCount })}
          </MetadataListItem>
          <MetadataListItem label={t("conn.dataDir")}>
            <Text type="code">{connections.dataDir}</Text>
          </MetadataListItem>
        </MetadataList>
      </Section>

      <Card padding={0}>
        <Stack paddingInline={5} paddingBlock={4} gap={1}>
          <Heading level={2}>{t("history")}</Heading>
          <Text type="supporting">{t("historyHint")}</Text>
        </Stack>
        <RevisionList revisions={revisions} canRestore={canRestore} />
      </Card>
    </Page>
  );
}
