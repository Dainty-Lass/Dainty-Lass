import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { auditSince, recentAudit } from "@/lib/audit/query";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { chatHourly, chatStats } from "@/lib/chat/store";
import { filterVersion } from "@/lib/chat/terms";
import { getController, LOCAL } from "@/lib/fleet/controllers";
import { gamesReport } from "@/lib/fleet/games";
import { currentScope, scopeIds } from "@/lib/fleet/scope";
import { fleetPerformance } from "@/lib/fleet/status";
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
  const me = await requirePage("overview:read");
  const [scope, { denied }] = await Promise.all([currentScope(), searchParams]);
  const ids = scopeIds(scope);
  const day = Date.now() - 86_400_000;
  return (
    <OverviewClient
      data={overview(ids)}
      perf={fleetPerformance(ids, can(me.role, "chat:read"))}
      scopeName={scope.kind === "one" ? (getController(scope.id)?.name ?? null) : null}
      reports={can(me.role, "reports:read") ? reportCounts(ids) : null}
      chat={can(me.role, "chat:read") ? chatStats(day, ids) : null}
      hourly={can(me.role, "chat:read") ? chatHourly(24, ids) : null}
      changesToday={can(me.role, "audit:read") ? auditSince(day, ids) : null}
      recent={can(me.role, "audit:read") ? recentAudit(10, ids) : null}
      filterVersion={filterVersion()}
      denied={denied === "1"}
      games={!ids || ids.includes(LOCAL) ? { report: gamesReport() } : null}
    />
  );
}
