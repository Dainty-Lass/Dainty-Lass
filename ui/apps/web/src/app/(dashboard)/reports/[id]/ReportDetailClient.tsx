"use client";

import { Avatar } from "@astryxdesign/core/Avatar";
import { Badge } from "@astryxdesign/core/Badge";
import { BreadcrumbItem, Breadcrumbs } from "@astryxdesign/core/Breadcrumbs";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Divider } from "@astryxdesign/core/Divider";
import { Grid } from "@astryxdesign/core/Grid";
import { Heading } from "@astryxdesign/core/Heading";
import { Link } from "@astryxdesign/core/Link";
import { MetadataList, MetadataListItem } from "@astryxdesign/core/MetadataList";
import { Selector } from "@astryxdesign/core/Selector";
import { Stack } from "@astryxdesign/core/Stack";
import { Tab, TabList } from "@astryxdesign/core/TabList";
import { Text } from "@astryxdesign/core/Text";
import { TextArea } from "@astryxdesign/core/TextArea";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { AuditTable } from "@/components/AuditTable";
import { ChatLog } from "@/components/ChatLog";
import { DirectMessages } from "@/components/DirectMessages";
import { Page } from "@/components/Page";
import { useAction } from "@/components/useAction";
import type { ChatLine } from "@/lib/chat/present";
import type { AuditEvent, Report, ReportNote, Staff } from "@/lib/db/schema";
import { REPORT_PRIORITIES, type ReportPriority } from "@/lib/reports/categories";
import type { ReportListItem } from "@/lib/reports/store";
import { claimAction, noteAction, priorityAction, releaseAction, reopenAction } from "../actions";
import { PRIORITY_COLOR, STATUS_BADGE } from "../ReportsClient";
import { ResolveDialog } from "./ResolveDialog";

export function ReportDetailClient({
  report,
  context,
  targetRecent,
  contextMinutes,
  related,
  history,
  reasons,
  me,
  can,
}: {
  report: Report & { assignee: Staff | null; notes: ReportNote[] };
  context: ChatLine[];
  targetRecent: ChatLine[];
  contextMinutes: number;
  related: ReportListItem[];
  history: AuditEvent[];
  reasons: string[];
  me: { id: number };
  can: { write: boolean; moderate: boolean; revert: boolean; readDirect: boolean };
}) {
  const t = useTranslations("reports");
  const td = useTranslations("direct");
  const { run } = useAction();
  const [tab, setTab] = useState("context");
  const [note, setNote] = useState("");
  const [resolving, setResolving] = useState<"resolved" | "dismissed" | null>(null);
  const closed = report.status === "resolved" || report.status === "dismissed";
  const mine = report.assigneeId === me.id;

  const actions = can.write ? (
    closed ? (
      <Button
        label={t("reopen")}
        clickAction={async () => void (await run(() => reopenAction(report.id), t("reopened")))}
      />
    ) : (
      <>
        {report.status === "open" && (
          <Button
            label={t("claim")}
            clickAction={async () =>
              void (await run(() => claimAction(report.id), t("claimed", { id: report.id })))
            }
          />
        )}
        {mine && (
          <Button
            label={t("release")}
            variant="ghost"
            clickAction={async () => void (await run(() => releaseAction(report.id)))}
          />
        )}
        <Button label={t("dismiss")} onClick={() => setResolving("dismissed")} />
        <Button label={t("resolve")} variant="primary" onClick={() => setResolving("resolved")} />
      </>
    )
  ) : undefined;

  return (
    <Page
      title={
        <Stack direction="horizontal" gap={2} align="center">
          {t("reportTitle", { id: report.id })}
          <Badge variant={STATUS_BADGE[report.status]} label={t(`statuses.${report.status}`)} />
        </Stack>
      }
      description={t("against", {
        name: report.targetName,
        category: t(`categories.${report.category}`),
      })}
      breadcrumbs={
        <Breadcrumbs variant="supporting">
          <BreadcrumbItem href="/reports">{t("title")}</BreadcrumbItem>
          <BreadcrumbItem>#{report.id}</BreadcrumbItem>
        </Breadcrumbs>
      }
      actions={actions}
    >
      <Grid columns={{ minWidth: 340, repeat: "fit" }} gap={4}>
        <Card padding={4}>
          <Stack gap={3}>
            <Heading level={2}>{t("whatHappened")}</Heading>
            <Text>{report.description}</Text>
            {report.resolution && (
              <>
                <Divider />
                <Text weight="medium">
                  {t(report.status === "dismissed" ? "dismissedBecause" : "resolution")}
                </Text>
                <Text>{report.resolution}</Text>
              </>
            )}
          </Stack>
        </Card>
        <Card padding={4}>
          <MetadataList label={{ position: "start", width: 130 }}>
            <MetadataListItem label={t("columns.player")}>
              <Link href={`/players/${encodeURIComponent(report.targetName)}`}>
                {report.targetName}
              </Link>
            </MetadataListItem>
            <MetadataListItem label={t("columns.reporter")}>
              {report.reporterName ?? t("anonymous")} · {t(`sources.${report.source}`)}
            </MetadataListItem>
            <MetadataListItem label={t("columns.filed")}>
              <Timestamp value={new Date(report.createdAt).toISOString()} format="date_time" />
            </MetadataListItem>
            <MetadataListItem label={t("columns.assignee")}>
              {report.assignee
                ? report.assignee.globalName || report.assignee.username
                : t("unassigned")}
            </MetadataListItem>
            <MetadataListItem label={t("columns.priority")}>
              {can.write && !closed ? (
                <Selector
                  label={t("columns.priority")}
                  isLabelHidden
                  size="sm"
                  variant="secondary"
                  value={report.priority}
                  options={REPORT_PRIORITIES.map((p) => ({
                    value: p,
                    label: t(`priorities.${p}`),
                  }))}
                  onChange={(v) => void run(() => priorityAction(report.id, v as ReportPriority))}
                />
              ) : (
                <Badge
                  variant={PRIORITY_COLOR[report.priority] === "red" ? "error" : "neutral"}
                  label={t(`priorities.${report.priority}`)}
                />
              )}
            </MetadataListItem>
            {report.context?.instanceId && (
              <MetadataListItem label={t("instance")}>{report.context.instanceId}</MetadataListItem>
            )}
          </MetadataList>
        </Card>
      </Grid>

      <Card padding={0}>
        <Stack paddingInline={4} paddingBlockStart={2}>
          <TabList value={tab} onChange={setTab} hasDivider>
            <Tab
              value="context"
              label={t("tabs.context")}
              endContent={<Badge label={String(context.length)} />}
            />
            <Tab value="recent" label={t("tabs.recent", { name: report.targetName })} />
            {can.readDirect && report.targetUserId && <Tab value="direct" label={td("tab")} />}
            <Tab
              value="related"
              label={t("tabs.related")}
              endContent={
                related.length ? (
                  <Badge label={String(related.length)} variant="warning" />
                ) : undefined
              }
            />
            <Tab value="history" label={t("tabs.history")} />
          </TabList>
        </Stack>
        {tab === "context" && (
          <Stack gap={0}>
            <Stack paddingInline={4} paddingBlock={2}>
              <Text type="supporting">{t("contextHint", { minutes: contextMinutes })}</Text>
            </Stack>
            <ChatLog lines={context} emptyTitle={t("noContext")} />
          </Stack>
        )}
        {tab === "recent" && <ChatLog lines={targetRecent} emptyTitle={t("noContext")} />}
        {tab === "direct" && report.targetUserId && (
          <DirectMessages
            controllerId={report.controllerId}
            userId={report.targetUserId}
            name={report.targetName}
            reportId={report.id}
          />
        )}
        {tab === "related" && (
          <Stack gap={1} padding={4}>
            {related.length === 0 ? (
              <Text type="supporting">{t("noRelated")}</Text>
            ) : (
              related.map((r) => (
                <Stack key={r.id} direction="horizontal" gap={2} align="center">
                  <Link href={`/reports/${r.id}`}>#{r.id}</Link>
                  <Text>{t(`categories.${r.category}`)}</Text>
                  <Badge variant={STATUS_BADGE[r.status]} label={t(`statuses.${r.status}`)} />
                  <Timestamp value={new Date(r.createdAt).toISOString()} format="relative_short" />
                </Stack>
              ))
            )}
          </Stack>
        )}
        {tab === "history" && <AuditTable events={history} canRevert={can.revert} />}
      </Card>

      <Card padding={4}>
        <Stack gap={3}>
          <Heading level={2}>{t("notes")}</Heading>
          {report.notes.length === 0 && <Text type="supporting">{t("noNotes")}</Text>}
          {report.notes.map((n) => (
            <Stack key={n.id} direction="horizontal" gap={3} align="start">
              <Avatar name={n.authorName} size="sm" />
              <Stack gap={0.5}>
                <Stack direction="horizontal" gap={2} align="center">
                  <Text weight="medium">{n.authorName}</Text>
                  <Timestamp value={new Date(n.at).toISOString()} format="relative_short" />
                </Stack>
                <Text>{n.body}</Text>
              </Stack>
            </Stack>
          ))}
          {can.write && (
            <Stack gap={2}>
              <TextArea
                label={t("addNote")}
                isLabelHidden
                placeholder={t("notePlaceholder")}
                value={note}
                onChange={setNote}
                rows={3}
                maxLength={2000}
              />
              <Stack direction="horizontal" justify="end">
                <Button
                  label={t("addNote")}
                  isDisabled={!note.trim()}
                  clickAction={async () => {
                    if ((await run(() => noteAction(report.id, note))) !== false) setNote("");
                  }}
                />
              </Stack>
            </Stack>
          )}
        </Stack>
      </Card>

      <ResolveDialog
        outcome={resolving}
        reportId={report.id}
        targetName={report.targetName}
        canModerate={can.moderate}
        reasons={reasons}
        onClose={() => setResolving(null)}
      />
    </Page>
  );
}
