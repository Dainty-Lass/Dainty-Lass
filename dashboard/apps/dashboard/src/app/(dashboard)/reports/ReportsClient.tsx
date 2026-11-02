"use client";

import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Link } from "@astryxdesign/core/Link";
import { Stack } from "@astryxdesign/core/Stack";
import { Switch } from "@astryxdesign/core/Switch";
import { Tab, TabList } from "@astryxdesign/core/TabList";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Token } from "@astryxdesign/core/Token";
import { useTranslations } from "next-intl";
import { ControllerLabel, useControllers } from "@/components/ControllerContext";
import { Page } from "@/components/Page";
import { useAction } from "@/components/useAction";
import { useQueryPatch } from "@/components/useQueryPatch";
import type { ReportStatus } from "@/lib/reports/categories";
import type { ReportListItem } from "@/lib/reports/store";
import { claimAction } from "./actions";

export const STATUS_BADGE = {
  open: "warning",
  claimed: "info",
  resolved: "success",
  dismissed: "neutral",
} as const;
export const PRIORITY_COLOR = { high: "red", normal: "default", low: "gray" } as const;

type Row = ReportListItem & Record<string, unknown>;

export function ReportsClient({
  reports,
  counts,
  status,
  mine,
  canWrite,
  meId,
}: {
  reports: ReportListItem[];
  counts: Record<ReportStatus, number>;
  status: ReportStatus | "active";
  mine: boolean;
  canWrite: boolean;
  meId: number;
}) {
  const t = useTranslations("reports");
  const { fleetWide } = useControllers();
  const patch = useQueryPatch();
  const { run } = useAction();

  return (
    <Page
      title={t("title")}
      description={t("description")}
      tabs={
        <TabList value={status} onChange={(v) => patch({ status: v === "active" ? null : v })}>
          <Tab
            value="active"
            label={t("tabs.active")}
            endContent={
              <Badge
                label={String(counts.open + counts.claimed)}
                variant={counts.open ? "warning" : "neutral"}
              />
            }
          />
          <Tab
            value="open"
            label={t("statuses.open")}
            endContent={<Badge label={String(counts.open)} />}
          />
          <Tab
            value="claimed"
            label={t("statuses.claimed")}
            endContent={<Badge label={String(counts.claimed)} />}
          />
          <Tab value="resolved" label={t("statuses.resolved")} />
          <Tab value="dismissed" label={t("statuses.dismissed")} />
        </TabList>
      }
      actions={
        <Switch
          label={t("assignedToMe")}
          value={mine}
          onChange={(on) => patch({ mine: on })}
          size="sm"
        />
      }
    >
      <Card padding={0}>
        {reports.length === 0 ? (
          <EmptyState
            title={status === "active" ? t("emptyQueue") : t("empty")}
            description={status === "active" ? t("emptyQueueHint") : undefined}
          />
        ) : (
          <Table<Row>
            data={reports as Row[]}
            idKey="id"
            hasHover
            verticalAlign="top"
            columns={[
              {
                key: "id",
                header: "#",
                width: pixel(80),
                renderCell: (r) => (
                  <Stack gap={1}>
                    <Link href={`/reports/${r.id}`}>#{r.id}</Link>
                    {r.priority !== "normal" && (
                      <Token
                        size="sm"
                        color={PRIORITY_COLOR[r.priority]}
                        label={t(`priorities.${r.priority}`)}
                      />
                    )}
                  </Stack>
                ),
              },
              {
                key: "targetName",
                header: t("columns.player"),
                width: pixel(180),
                renderCell: (r) => (
                  <Stack gap={0.5}>
                    <Link href={`/players/${encodeURIComponent(r.targetName)}?c=${r.controllerId}`}>
                      {r.targetName}
                    </Link>
                    {fleetWide && <ControllerLabel id={r.controllerId} />}
                    {r.priorAgainstTarget > 0 && (
                      <Text type="supporting">{t("prior", { count: r.priorAgainstTarget })}</Text>
                    )}
                  </Stack>
                ),
              },
              {
                key: "description",
                header: t("columns.description"),
                width: proportional(3),
                renderCell: (r) => (
                  <Stack gap={1}>
                    <Text weight="medium">{t(`categories.${r.category}`)}</Text>
                    <Text maxLines={2} type="supporting">
                      {r.description}
                    </Text>
                  </Stack>
                ),
              },
              {
                key: "reporterName",
                header: t("columns.reporter"),
                width: pixel(160),
                renderCell: (r) => (
                  <Stack gap={0.5}>
                    <Text>{r.reporterName ?? t("anonymous")}</Text>
                    <Text type="supporting">{t(`sources.${r.source}`)}</Text>
                  </Stack>
                ),
              },
              {
                key: "status",
                header: t("columns.status"),
                width: pixel(160),
                renderCell: (r) => (
                  <Stack gap={0.5}>
                    <Badge variant={STATUS_BADGE[r.status]} label={t(`statuses.${r.status}`)} />
                    {r.assigneeName && <Text type="supporting">{r.assigneeName}</Text>}
                  </Stack>
                ),
              },
              {
                key: "createdAt",
                header: t("columns.filed"),
                width: pixel(120),
                renderCell: (r) => (
                  <Timestamp value={new Date(r.createdAt).toISOString()} format="relative_short" />
                ),
              },
              {
                key: "act",
                header: "",
                width: pixel(110),
                align: "end",
                renderCell: (r) =>
                  canWrite && r.status === "open" ? (
                    <Button
                      label={t("claim")}
                      size="sm"
                      clickAction={async () =>
                        void (await run(() => claimAction(r.id), t("claimed", { id: r.id })))
                      }
                    />
                  ) : r.assigneeId === meId && r.status === "claimed" ? (
                    <Button
                      label={t("review")}
                      size="sm"
                      variant="primary"
                      href={`/reports/${r.id}`}
                    />
                  ) : null,
              },
            ]}
          />
        )}
      </Card>
    </Page>
  );
}
