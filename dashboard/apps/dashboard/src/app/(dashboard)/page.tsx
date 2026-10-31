import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { auditSince, recentAudit } from "@/lib/audit/query";
import { requirePage } from "@/lib/auth/guard";
import { chatHourly, chatStats } from "@/lib/chat/store";
import { filterVersion } from "@/lib/chat/terms";
import { getController } from "@/lib/fleet/controllers";
import { currentScope, scopeIds } from "@/lib/fleet/scope";
import { overview } from "@/lib/health";
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
  await requirePage("overview:read");
  const [scope, { denied }] = await Promise.all([currentScope(), searchParams]);
  const ids = scopeIds(scope);
  const day = Date.now() - 86_400_000;
  return (
    <OverviewClient
      data={overview(ids)}
      scopeName={scope.kind === "one" ? (getController(scope.id)?.name ?? null) : null}
      reports={reportCounts(ids)}
      chat={chatStats(day, ids)}
      hourly={chatHourly(24, ids)}
      changesToday={auditSince(day, ids)}
      recent={recentAudit(10, ids)}
      filterVersion={filterVersion()}
      denied={denied === "1"}
    />
  );
}
