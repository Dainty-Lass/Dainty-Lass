import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { REPORT_STATUSES, type ReportStatus } from "@/lib/reports/categories";
import { listReports, reportCounts } from "@/lib/reports/store";
import { ReportsClient } from "./ReportsClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("reports"))("title") };
}

export default async function ReportsPage({
  searchParams,
}: {
  searchParams: Promise<{ status?: string; mine?: string }>;
}) {
  const me = await requirePage("reports:read");
  const sp = await searchParams;
  const status: ReportStatus | "active" = REPORT_STATUSES.find((s) => s === sp.status) ?? "active";
  const mine = sp.mine === "1";
  return (
    <ReportsClient
      reports={listReports({ status, assigneeId: mine ? me.id : undefined })}
      counts={reportCounts()}
      status={status}
      mine={mine}
      canWrite={can(me.role, "reports:write")}
      meId={me.id}
    />
  );
}
