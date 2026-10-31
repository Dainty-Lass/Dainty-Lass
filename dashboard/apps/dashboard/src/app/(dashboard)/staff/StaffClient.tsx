"use client";

import { Avatar } from "@astryxdesign/core/Avatar";
import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { Heading } from "@astryxdesign/core/Heading";
import { Selector } from "@astryxdesign/core/Selector";
import { Stack } from "@astryxdesign/core/Stack";
import { Switch } from "@astryxdesign/core/Switch";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Token } from "@astryxdesign/core/Token";
import { UserPlus } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { Page } from "@/components/Page";
import { RevisionList, type RevisionRow } from "@/components/RevisionList";
import { useAction } from "@/components/useAction";
import { canAssign, ROLES, type Role } from "@/lib/auth/permissions";
import type { Staff } from "@/lib/db/schema";
import { addStaffAction, updateStaffAction } from "./actions";

type Member = Staff & { avatarUrl: string | null };
type Row = Member & Record<string, unknown>;

export function StaffClient({
  staff,
  me,
  canWrite,
  canRestore,
  guildSync,
  revisions,
}: {
  staff: Member[];
  me: { id: number; role: Role };
  canWrite: boolean;
  canRestore: boolean;
  guildSync: boolean;
  revisions: RevisionRow[];
}) {
  const t = useTranslations("staff");
  const tc = useTranslations("common");
  const { run } = useAction();
  const [adding, setAdding] = useState(false);
  const [discordId, setDiscordId] = useState("");
  const [username, setUsername] = useState("");
  const [role, setRole] = useState<Role>("moderator");
  const [note, setNote] = useState("");
  const assignable = ROLES.filter((r) => canAssign(me.role, r));

  return (
    <Page
      title={t("title")}
      description={t("description")}
      actions={
        canWrite ? (
          <Button
            label={t("add")}
            variant="primary"
            icon={<UserPlus size={16} />}
            onClick={() => setAdding(true)}
          />
        ) : undefined
      }
    >
      {guildSync && (
        <Banner
          status="info"
          title={t("guildSync")}
          description={t("guildSyncHint")}
          collapsible={false}
        />
      )}
      <Card padding={0}>
        <Table<Row>
          data={staff as Row[]}
          idKey="id"
          hasHover
          columns={[
            {
              key: "username",
              header: t("columns.member"),
              width: proportional(2),
              renderCell: (s) => (
                <Stack direction="horizontal" gap={2} align="center">
                  <Avatar
                    name={s.globalName || s.username}
                    src={s.avatarUrl ?? undefined}
                    size="sm"
                  />
                  <Stack gap={0}>
                    <Text weight="medium" hasStrikethrough={s.disabled}>
                      {s.globalName || s.username}
                      {s.id === me.id ? ` ${t("you")}` : ""}
                    </Text>
                    <Text type="supporting" size="xsm">
                      {s.discordId}
                    </Text>
                  </Stack>
                </Stack>
              ),
            },
            {
              key: "role",
              header: t("columns.role"),
              width: pixel(170),
              renderCell: (s) =>
                canWrite && s.id !== me.id && canAssign(me.role, s.role) ? (
                  <Selector
                    label={t("columns.role")}
                    isLabelHidden
                    size="sm"
                    variant="secondary"
                    value={s.role}
                    options={assignable.map((r) => ({ value: r, label: tc(`roles.${r}`) }))}
                    onChange={(v) =>
                      void run(() => updateStaffAction(s.id, { role: v as Role }), t("updated"))
                    }
                  />
                ) : (
                  <Badge
                    variant={s.role === "owner" ? "purple" : "neutral"}
                    label={tc(`roles.${s.role}`)}
                  />
                ),
            },
            {
              key: "roleSource",
              header: t("columns.source"),
              width: pixel(120),
              renderCell: (s) => <Token size="sm" label={t(`sources.${s.roleSource}`)} />,
            },
            {
              key: "lastSeenAt",
              header: t("columns.lastSeen"),
              width: pixel(140),
              renderCell: (s) =>
                s.lastSeenAt ? (
                  <Timestamp value={new Date(s.lastSeenAt).toISOString()} format="relative_short" />
                ) : (
                  t("never")
                ),
            },
            {
              key: "note",
              header: t("columns.note"),
              width: proportional(1),
              renderCell: (s) => <Text type="supporting">{s.note ?? ""}</Text>,
            },
            {
              key: "disabled",
              header: t("columns.access"),
              width: pixel(110),
              renderCell: (s) => (
                <Switch
                  label={t("columns.access")}
                  isLabelHidden
                  size="sm"
                  value={!s.disabled}
                  isDisabled={!canWrite || s.id === me.id || !canAssign(me.role, s.role)}
                  changeAction={async (on) =>
                    void (await run(
                      () => updateStaffAction(s.id, { disabled: !on }),
                      on ? t("enabled") : t("disabled"),
                    ))
                  }
                />
              ),
            },
          ]}
        />
      </Card>

      <Card padding={0}>
        <Stack paddingInline={4} paddingBlock={3} gap={1}>
          <Heading level={2}>{t("history")}</Heading>
          <Text type="supporting">{t("historyHint")}</Text>
        </Stack>
        <RevisionList revisions={revisions} canRestore={canRestore} />
      </Card>

      <Dialog isOpen={adding} onOpenChange={setAdding} purpose="form" width={480}>
        <Stack gap={4}>
          <DialogHeader title={t("addTitle")} subtitle={t("addExplain")} onOpenChange={setAdding} />
          <Stack gap={4} paddingInline={4}>
            <TextInput
              label={t("discordId")}
              description={t("discordIdHint")}
              value={discordId}
              onChange={setDiscordId}
              isRequired
              status={
                discordId && !/^\d{17,20}$/.test(discordId.trim())
                  ? { type: "error", message: t("discordIdInvalid") }
                  : undefined
              }
            />
            <TextInput
              label={t("displayName")}
              value={username}
              onChange={setUsername}
              isOptional
            />
            <Selector
              label={t("columns.role")}
              value={role}
              onChange={(v) => setRole(v as Role)}
              options={assignable.map((r) => ({
                value: r,
                label: tc(`roles.${r}`),
                description: t(`roleHelp.${r}`),
              }))}
              width="100%"
            />
            <TextInput label={t("columns.note")} value={note} onChange={setNote} isOptional />
          </Stack>
          <Stack direction="horizontal" gap={2} justify="end" paddingInline={4} paddingBlockEnd={4}>
            <Button label={tc("cancel")} variant="ghost" onClick={() => setAdding(false)} />
            <Button
              label={t("add")}
              variant="primary"
              isDisabled={!/^\d{17,20}$/.test(discordId.trim())}
              clickAction={async () => {
                if (
                  (await run(() => addStaffAction(discordId, username, role, note), t("added"))) !==
                  false
                ) {
                  setAdding(false);
                  setDiscordId("");
                  setUsername("");
                  setNote("");
                }
              }}
            />
          </Stack>
        </Stack>
      </Dialog>
    </Page>
  );
}
