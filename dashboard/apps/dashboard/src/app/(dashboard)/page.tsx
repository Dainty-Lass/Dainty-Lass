import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { auditSince, recentAudit } from "@/lib/audit/query";
import { requirePage } from "@/lib/auth/guard";
import { chatHourly, chatStats } from "@/lib/chat/store";
import { filterVersion } from "@/lib/chat/terms";
import { health } from "@/lib/health";
import { op } from "@/lib/players";
import { reportCounts } from "@/lib/reports/store";
import { OverviewClient } from "./OverviewClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("overview"))("title") };
}

export default async function OverviewPage({
  searchParams,
}: {
  searchParams: Promise<{ denied?: string }>;
}) {
  const me = await requirePage("overview:read");
  const day = Date.now() - 86_400_000;
  const [h, { denied }] = await Promise.all([health(op(me)), searchParams]);
  return (
    <OverviewClient
      health={h}
      reports={reportCounts()}
      chat={chatStats(day)}
      hourly={chatHourly(24)}
      changesToday={auditSince(day)}
      recent={recentAudit(10)}
      filterVersion={filterVersion()}
      denied={denied === "1"}
    />
  );
}
