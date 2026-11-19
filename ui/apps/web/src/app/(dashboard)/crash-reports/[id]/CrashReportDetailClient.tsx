"use client";

import { BreadcrumbItem, Breadcrumbs } from "@astryxdesign/core/Breadcrumbs";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { Link } from "@astryxdesign/core/Link";
import { MetadataList, MetadataListItem } from "@astryxdesign/core/MetadataList";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { useTranslations } from "next-intl";
import { ControllerLabel, useControllers } from "@/components/ControllerContext";
import { Page } from "@/components/Page";
import type { CrashReport } from "@/lib/db/schema";

export function CrashReportDetailClient({ report }: { report: CrashReport }) {
  const t = useTranslations("crashReports");
  const { fleetWide } = useControllers();
  const orUnknown = (v: string | null) => v ?? t("unknown");

  return (
    <Page
      title={t("reportTitle", { crashId: report.crashId })}
      breadcrumbs={
        <Breadcrumbs variant="supporting">
          <BreadcrumbItem href="/crash-reports">{t("title")}</BreadcrumbItem>
          <BreadcrumbItem>{report.crashId}</BreadcrumbItem>
        </Breadcrumbs>
      }
    >
      <Stack gap={4}>
        <Card padding={4}>
          <MetadataList label={{ position: "start", width: 160 }}>
            <MetadataListItem label={t("columns.player")}>
              <Stack gap={0.5}>
                <Link
                  href={`/players/${encodeURIComponent(report.userName)}?c=${report.controllerId}`}
                >
                  {report.userName}
                </Link>
                {fleetWide && <ControllerLabel id={report.controllerId} />}
              </Stack>
            </MetadataListItem>
            <MetadataListItem label={t("columns.received")}>
              <Timestamp value={new Date(report.receivedAt).toISOString()} format="date_time" />
            </MetadataListItem>
            <MetadataListItem label={t("columns.gameVersion")}>
              {orUnknown(report.gameVersion)}
            </MetadataListItem>
            <MetadataListItem label={t("columns.launcherVersion")}>
              {orUnknown(report.launcherVersion)}
            </MetadataListItem>
            <MetadataListItem label={t("columns.exitCode")}>
              {orUnknown(report.exitCode)}
            </MetadataListItem>
            <MetadataListItem label={t("columns.serverPath")}>
              <Text type="code">{report.serverPath}</Text>
            </MetadataListItem>
          </MetadataList>
        </Card>
        <Heading level={2}>{t("filesHeading")}</Heading>
        {report.files.length === 0 ? (
          <Card padding={0}>
            <EmptyState title={t("noFiles")} />
          </Card>
        ) : (
          report.files.map((f) => (
            <Card key={`${f.name}:${f.sha256}`} padding={4}>
              <Stack gap={3}>
                <MetadataList label={{ position: "start", width: 160 }}>
                  <MetadataListItem label={t("columns.name")}>
                    <Text weight="medium">{f.name}</Text>
                  </MetadataListItem>
                  <MetadataListItem label={t("columns.size")}>
                    {t("bytes", { size: f.size })}
                  </MetadataListItem>
                  <MetadataListItem label={t("columns.sha256")}>
                    <Text type="code">{f.sha256}</Text>
                  </MetadataListItem>
                </MetadataList>
                {f.text === null ? (
                  <Text type="supporting">{t("noText")}</Text>
                ) : (
                  <pre className="dainty-console">{f.text}</pre>
                )}
              </Stack>
            </Card>
          ))
        )}
      </Stack>
    </Page>
  );
}
