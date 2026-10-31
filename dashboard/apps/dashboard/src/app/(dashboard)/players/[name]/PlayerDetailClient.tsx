"use client";

import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { BreadcrumbItem, Breadcrumbs } from "@astryxdesign/core/Breadcrumbs";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { DropdownMenu } from "@astryxdesign/core/DropdownMenu";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Grid } from "@astryxdesign/core/Grid";
import { Link } from "@astryxdesign/core/Link";
import { MetadataList, MetadataListItem } from "@astryxdesign/core/MetadataList";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Tab, TabList } from "@astryxdesign/core/TabList";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Token } from "@astryxdesign/core/Token";
import { Ban, Clock, DoorOpen, Flag, ShieldCheck } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { AuditTable } from "@/components/AuditTable";
import { ChatLog } from "@/components/ChatLog";
import { Page } from "@/components/Page";
import { useAction } from "@/components/useAction";
import type { ChatLine } from "@/lib/chat/present";
import type { AuditEvent, Report } from "@/lib/db/schema";
import type { GamePlayer } from "@/lib/game/admin-api";
import type { BackendAccount } from "@/lib/game/backend";
import { liftTimeoutAction } from "../actions";
import { AccessDialog } from "./AccessDialog";
import { FileReportDialog } from "./FileReportDialog";
import { ModerationDialog, type ModerationKind } from "./ModerationDialog";

const STATUS_BADGE = {
  ACTIVE: "success",
  BANNED: "error",
  DISABLED: "neutral",
  UNKNOWN: "neutral",
} as const;
const REPORT_BADGE = {
  open: "warning",
  claimed: "info",
  resolved: "success",
  dismissed: "neutral",
} as const;

export function PlayerDetailClient({
  name,
  userId,
  player,
  account,
  gameFailure,
  notFound,
  chat,
  reports,
  history,
  reasons,
  can,
  backLabel,
}: {
  name: string;
  userId: string | null;
  player: GamePlayer | null;
  account: BackendAccount | null;
  gameFailure: string | null;
  notFound: boolean;
  chat: ChatLine[];
  reports: Report[];
  history: AuditEvent[];
  reasons: string[];
  can: { moderate: boolean; access: boolean; roles: boolean; report: boolean; revert: boolean };
  backLabel: string;
}) {
  const t = useTranslations("players");
  const tm = useTranslations("moderation");
  const te = useTranslations("errors");
  const tr = useTranslations("reports");
  const { run } = useAction();
  const [tab, setTab] = useState("chat");
  const [dialog, setDialog] = useState<ModerationKind | null>(null);
  const [access, setAccess] = useState(false);
  const [reporting, setReporting] = useState(false);

  const r = player?.restriction;
  const restricted = r && r.kind !== "DISABLED";
  const playable = Boolean(player) && !player?.admin;

  const actions =
    can.moderate && playable ? (
      <>
        <Button
          label={tm("actions.kick")}
          icon={<DoorOpen size={16} />}
          onClick={() => setDialog("kick")}
          isDisabled={!player?.online}
          tooltip={player?.online ? undefined : tm("kickOffline")}
        />
        {r?.kind === "TIMEOUT" ? (
          <Button
            label={tm("actions.liftTimeout")}
            icon={<Clock size={16} />}
            clickAction={async () =>
              void (await run(() => liftTimeoutAction(name), tm("done.liftTimeout", { name })))
            }
          />
        ) : (
          <Button
            label={tm("actions.timeout")}
            icon={<Clock size={16} />}
            onClick={() => setDialog("timeout")}
            isDisabled={r?.kind === "BAN"}
          />
        )}
        {restricted ? (
          <Button
            label={tm("actions.unban")}
            variant="primary"
            icon={<ShieldCheck size={16} />}
            onClick={() => setDialog("unban")}
          />
        ) : (
          <Button
            label={tm("actions.ban")}
            variant="destructive"
            icon={<Ban size={16} />}
            onClick={() => setDialog("ban")}
          />
        )}
        <DropdownMenu
          button={{ label: t("more"), variant: "ghost" }}
          items={[
            ...(can.report
              ? [{ label: t("fileReport"), icon: Flag, onClick: () => setReporting(true) }]
              : []),
            ...(can.access && account
              ? [{ label: t("editAccess"), onClick: () => setAccess(true) }]
              : []),
          ]}
        />
      </>
    ) : can.report ? (
      <Button
        label={t("fileReport")}
        icon={<Flag size={16} />}
        onClick={() => setReporting(true)}
      />
    ) : undefined;

  return (
    <Page
      title={
        <Stack direction="horizontal" gap={2} align="center">
          {name}
          {player && (
            <Badge variant={STATUS_BADGE[player.status]} label={t(`gameStatus.${player.status}`)} />
          )}
          {player?.admin && <Badge variant="purple" label={t("adminAccount")} />}
        </Stack>
      }
      description={
        player ? (
          <Stack direction="horizontal" gap={1} align="center">
            <StatusDot
              variant={player.online ? "success" : "neutral"}
              label={player.online ? t("online") : t("offline")}
            />
            {player.online
              ? player.location
                ? t("onlineIn", { place: player.location.title })
                : t("onlineMenus")
              : t("offline")}
          </Stack>
        ) : undefined
      }
      breadcrumbs={
        <Breadcrumbs variant="supporting">
          <BreadcrumbItem href="/players">{backLabel}</BreadcrumbItem>
          <BreadcrumbItem>{name}</BreadcrumbItem>
        </Breadcrumbs>
      }
      actions={actions}
    >
      {gameFailure && (
        <Banner
          status="warning"
          title={te(gameFailure as "unknown", { detail: "DASHBOARD_ADMIN_API_KEY" })}
          description={t("partialData")}
          collapsible={false}
        />
      )}
      {notFound && <Banner status="error" title={t("notFound", { name })} collapsible={false} />}
      {player?.admin && <Banner status="info" title={t("protected")} collapsible={false} />}

      {restricted && r && (
        <Banner
          status="error"
          title={
            r.kind === "TIMEOUT" && r.until
              ? t("timedOutUntil", { until: new Date(r.until) })
              : t("bannedForGood")
          }
          description={r.reason ?? undefined}
          collapsible={false}
        />
      )}

      <Grid columns={{ minWidth: 320, repeat: "fit" }} gap={4}>
        <Card padding={4}>
          <MetadataList title={t("game")} label={{ position: "start", width: 140 }}>
            <MetadataListItem label={t("fields.userId")}>
              <Text type="code">{userId ?? "—"}</Text>
            </MetadataListItem>
            <MetadataListItem label={t("fields.joined")}>
              {player?.createdAt ? <Timestamp value={player.createdAt} format="date_time" /> : "—"}
            </MetadataListItem>
            <MetadataListItem label={t("fields.lastLogin")}>
              {player?.lastLoginAt ? <Timestamp value={player.lastLoginAt} format="auto" /> : "—"}
            </MetadataListItem>
            <MetadataListItem label={t("fields.location")}>
              {player?.location ? `${player.location.title} (${player.location.instanceId})` : "—"}
            </MetadataListItem>
            {r && (
              <MetadataListItem label={t("fields.restrictedBy")}>
                {r.by ?? "—"}
                {r.since && (
                  <>
                    {" · "}
                    <Timestamp value={r.since} format="auto" />
                  </>
                )}
              </MetadataListItem>
            )}
          </MetadataList>
        </Card>
        <Card padding={4}>
          <MetadataList title={t("account")} label={{ position: "start", width: 140 }}>
            <MetadataListItem label={t("fields.access")}>
              {account ? t(`statuses.${account.status}`) : "—"}
            </MetadataListItem>
            <MetadataListItem label={t("fields.approval")}>
              {account ? t(`approval.${account.approvalStatus}`) : "—"}
            </MetadataListItem>
            <MetadataListItem label={t("fields.roles")}>
              <Stack direction="horizontal" gap={1} wrap="wrap">
                {account?.roles.length
                  ? account.roles.map((role) => <Token key={role} size="sm" label={role} />)
                  : "—"}
              </Stack>
            </MetadataListItem>
            <MetadataListItem label={t("fields.reports")}>
              {t("reportCount", { count: reports.length })}
            </MetadataListItem>
            {account?.approvalReason && (
              <MetadataListItem label={t("fields.accessReason")}>
                {account.approvalReason}
              </MetadataListItem>
            )}
          </MetadataList>
        </Card>
      </Grid>

      <Card padding={0}>
        <Stack paddingInline={4} paddingBlockStart={2}>
          <TabList value={tab} onChange={setTab} hasDivider>
            <Tab
              value="chat"
              label={t("tabs.chat")}
              endContent={<Badge label={String(chat.length)} />}
            />
            <Tab
              value="reports"
              label={t("tabs.reports")}
              endContent={reports.length ? <Badge label={String(reports.length)} /> : undefined}
            />
            <Tab value="history" label={t("tabs.history")} />
          </TabList>
        </Stack>
        {tab === "chat" && (
          <Stack gap={0}>
            <ChatLog lines={chat} emptyTitle={t("noChat")} />
            {userId && chat.length >= 100 && (
              <Stack padding={3} align="center">
                <Link href={`/chat?user=${encodeURIComponent(userId)}`}>{t("allChat")}</Link>
              </Stack>
            )}
          </Stack>
        )}
        {tab === "reports" &&
          (reports.length === 0 ? (
            <EmptyState isCompact title={t("noReports")} />
          ) : (
            <Table<Report & Record<string, unknown>>
              data={reports as (Report & Record<string, unknown>)[]}
              idKey="id"
              density="compact"
              hasHover
              columns={[
                {
                  key: "id",
                  header: "#",
                  width: pixel(70),
                  renderCell: (x) => <Link href={`/reports/${x.id}`}>#{x.id}</Link>,
                },
                {
                  key: "category",
                  header: tr("columns.category"),
                  width: pixel(170),
                  renderCell: (x) => tr(`categories.${x.category}`),
                },
                {
                  key: "description",
                  header: tr("columns.description"),
                  width: proportional(3),
                  renderCell: (x) => <Text maxLines={2}>{x.description}</Text>,
                },
                {
                  key: "status",
                  header: tr("columns.status"),
                  width: pixel(120),
                  renderCell: (x) => (
                    <Badge variant={REPORT_BADGE[x.status]} label={tr(`statuses.${x.status}`)} />
                  ),
                },
                {
                  key: "createdAt",
                  header: tr("columns.filed"),
                  width: pixel(120),
                  renderCell: (x) => (
                    <Timestamp
                      value={new Date(x.createdAt).toISOString()}
                      format="relative_short"
                    />
                  ),
                },
              ]}
            />
          ))}
        {tab === "history" && (
          <AuditTable events={history} canRevert={can.revert} emptyTitle={t("noHistory")} />
        )}
      </Card>

      <ModerationDialog
        kind={dialog}
        username={name}
        reasons={reasons}
        onClose={() => setDialog(null)}
      />
      {account && (
        <AccessDialog
          isOpen={access}
          account={account}
          canRoles={can.roles}
          reasons={reasons}
          onClose={() => setAccess(false)}
        />
      )}
      <FileReportDialog
        isOpen={reporting}
        target={{ userId, name }}
        onClose={() => setReporting(false)}
      />
    </Page>
  );
}
