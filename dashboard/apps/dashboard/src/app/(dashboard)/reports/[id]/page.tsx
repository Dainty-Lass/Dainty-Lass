import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { getTranslations } from "next-intl/server";
import { queryAudit } from "@/lib/audit/query";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { present } from "@/lib/chat/present";
import { chatAround, searchChat } from "@/lib/chat/store";
import { getReport, listReports } from "@/lib/reports/store";
import { getSettings } from "@/lib/settings";
import { ReportDetailClient } from "./ReportDetailClient";

export async function generateMetadata({
  params,
}: {
  params: Promise<{ id: string }>;
}): Promise<Metadata> {
  const t = await getTranslations("reports");
  return { title: t("reportTitle", { id: (await params).id }) };
}

export default async function ReportPage({ params }: { params: Promise<{ id: string }> }) {
  const me = await requirePage("reports:read");
  const id = Number((await params).id);
  const report = Number.isInteger(id) ? getReport(id) : undefined;
  if (!report) notFound();

  const { reportContextMinutes, moderationReasons } = getSettings();
  // The room around the moment when the report says where; otherwise the target's own recent lines.
  const at = report.context?.at ?? report.createdAt;
  const context = report.context?.roomId
    ? chatAround(at, reportContextMinutes, {
        controllerId: report.controllerId,
        roomId: report.context.roomId,
      })
    : report.targetUserId
      ? chatAround(at, reportContextMinutes, {
          controllerId: report.controllerId,
          userId: report.targetUserId,
        })
      : [];
  const targetRecent = report.targetUserId
    ? searchChat({ userId: report.targetUserId, controllerIds: [report.controllerId], limit: 30 })
        .messages
    : [];

  return (
    <ReportDetailClient
      report={report}
      context={present(context)}
      targetRecent={present(targetRecent)}
      contextMinutes={reportContextMinutes}
      related={listReports({ targetUserId: report.targetUserId ?? undefined }).filter(
        (r) => r.id !== report.id && report.targetUserId,
      )}
      history={
        queryAudit({ targetType: "report", targetId: String(report.id), pageSize: 50 }).events
      }
      reasons={moderationReasons}
      me={{ id: me.id }}
      can={{
        write: can(me.role, "reports:write"),
        moderate: can(me.role, "players:moderate"),
        revert: can(me.role, "audit:revert"),
      }}
    />
  );
}
