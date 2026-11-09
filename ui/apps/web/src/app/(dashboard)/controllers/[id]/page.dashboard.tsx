import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { queryAudit } from "@/lib/audit/query";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { config } from "@/lib/config";
import { getController, LOCAL } from "@/lib/fleet/controllers";
import { gamesReport } from "@/lib/fleet/games";
import { readLogSettings } from "@/lib/fleet/log-settings";
import { currentScope } from "@/lib/fleet/scope";
import { fleetPerformance, recentSamples, viewOf } from "@/lib/fleet/status";
import { reportCounts } from "@/lib/reports/store";
import { ControllerDetailClient } from "./ControllerDetailClient";

export async function generateMetadata({
  params,
}: {
  params: Promise<{ id: string }>;
}): Promise<Metadata> {
  return { title: getController((await params).id)?.name ?? "—" };
}

export default async function ControllerPage({ params }: { params: Promise<{ id: string }> }) {
  const me = await requirePage("controllers:read");
  const { id } = await params;
  const controller = getController(id);
  if (!controller) notFound();
  const scope = await currentScope();
  // A week of minute samples, folded to hourly peaks for the chart.
  const hourly = new Map<number, { online: number; hunts: number }>();
  for (const s of recentSamples(id, Date.now() - 7 * 86_400_000)) {
    const h = Math.floor(s.at / 3_600_000) * 3_600_000;
    const cur = hourly.get(h) ?? { online: 0, hunts: 0 };
    hourly.set(h, {
      online: Math.max(cur.online, s.online),
      hunts: Math.max(cur.hunts, s.huntsRunning),
    });
  }
  const canWrite = can(me.role, "controllers:write");
  // Only for those who may change them: the collector's address is the host's business
  const view = viewOf(controller);
  const logs = canWrite && view.state !== "pending" ? await readLogSettings(me, id) : null;
  return (
    <ControllerDetailClient
      controller={view}
      perf={fleetPerformance([id], can(me.role, "chat:read"))}
      history={Array.from({ length: 168 }, (_, i) => {
        const at = Math.floor(Date.now() / 3_600_000) * 3_600_000 - (167 - i) * 3_600_000;
        return { at, ...(hourly.get(at) ?? { online: 0, hunts: 0 }) };
      })}
      reports={can(me.role, "reports:read") ? reportCounts([id]) : null}
      audit={
        can(me.role, "audit:read") ? queryAudit({ controllerIds: [id], pageSize: 25 }).events : null
      }
      inScope={scope.kind === "one" && scope.id === id}
      canWrite={canWrite}
      logs={logs}
      games={id === LOCAL ? { report: gamesReport() } : null}
      canRevert={can(me.role, "audit:revert")}
      dashboardUrl={config.baseUrl}
      insecure={!config.baseUrl.startsWith("https:")}
    />
  );
}
