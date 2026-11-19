"use client";

import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Link } from "@astryxdesign/core/Link";
import { Stack } from "@astryxdesign/core/Stack";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { useTranslations } from "next-intl";
import { ControllerLabel, useControllers } from "@/components/ControllerContext";
import { Page } from "@/components/Page";
import type { CrashReportListItem } from "@/lib/crash-reports/store";

type Row = CrashReportListItem & Record<string, unknown>;

export function CrashReportsClient({ reports }: { reports: CrashReportListItem[] }) {
  const t = useTranslations("crashReports");
  const { fleetWide } = useControllers();
  const orUnknown = (v: string | null) => v ?? t("unknown");

  return (
    <Page title={t("title")} description={t("description")}>
      <Card padding={0}>
        {reports.length === 0 ? (
          <EmptyState title={t("empty")} description={t("emptyHint")} />
        ) : (
          <Table<Row>
            data={reports as Row[]}
            idKey="id"
            hasHover
            verticalAlign="top"
            columns={[
              {
                key: "receivedAt",
                header: t("columns.received"),
                width: pixel(130),
                renderCell: (r) => (
                  <Timestamp value={new Date(r.receivedAt).toISOString()} format="relative_short" />
                ),
              },
              {
                key: "userName",
                header: t("columns.player"),
                width: pixel(180),
                renderCell: (r) => (
                  <Stack gap={0.5}>
                    <Link href={`/players/${encodeURIComponent(r.userName)}?c=${r.controllerId}`}>
                      {r.userName}
                    </Link>
                    {fleetWide && <ControllerLabel id={r.controllerId} />}
                  </Stack>
                ),
              },
              {
                key: "crashId",
                header: t("columns.crashId"),
                width: proportional(2),
                renderCell: (r) => <Link href={`/crash-reports/${r.id}`}>{r.crashId}</Link>,
              },
              {
                key: "gameVersion",
                header: t("columns.gameVersion"),
                width: pixel(140),
                renderCell: (r) => <Text>{orUnknown(r.gameVersion)}</Text>,
              },
              {
                key: "launcherVersion",
                header: t("columns.launcherVersion"),
                width: pixel(140),
                renderCell: (r) => <Text>{orUnknown(r.launcherVersion)}</Text>,
              },
              {
                key: "exitCode",
                header: t("columns.exitCode"),
                width: pixel(120),
                renderCell: (r) => <Text>{orUnknown(r.exitCode)}</Text>,
              },
              {
                key: "fileCount",
                header: t("columns.files"),
                width: pixel(100),
                renderCell: (r) => <Text>{t("fileCount", { count: r.fileCount })}</Text>,
              },
            ]}
          />
        )}
      </Card>
    </Page>
  );
}
