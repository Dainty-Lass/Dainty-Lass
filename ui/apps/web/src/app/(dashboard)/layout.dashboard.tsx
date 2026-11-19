import type { ReactNode } from "react";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { currentScope, scopeIds } from "@/lib/fleet/scope";
import { fleet } from "@/lib/fleet/status";
import { pendingCount } from "@/lib/onboarding";
import { reportCounts } from "@/lib/reports/store";
import { getSettings } from "@/lib/settings";
import { DashboardShell } from "./DashboardShell";

export default async function DashboardLayout({ children }: { children: ReactNode }) {
  const me = await requirePage("overview:read");
  const scope = await currentScope();
  const counts = can(me.role, "reports:read") ? reportCounts(scopeIds(scope)) : { open: 0 };
  const { serverName } = getSettings();
  const controllers = fleet();
  return (
    <DashboardShell
      serverName={serverName}
      me={{
        name: me.displayName || me.globalName || me.username,
        role: me.role,
        avatar: me.avatar ?? undefined,
      }}
      openReports={counts.open}
      pending={can(me.role, "staff:read") ? pendingCount() : 0}
      scope={scope.kind === "one" ? scope.id : "all"}
      controllers={controllers.map((c) => ({ id: c.id, name: c.name, state: c.state }))}
      nav={{
        reports: can(me.role, "reports:read"),
        crashReports: can(me.role, "reports:read"),
        chat: can(me.role, "chat:read"),
        audit: can(me.role, "audit:read"),
        controllers: can(me.role, "controllers:read"),
        staff: can(me.role, "staff:read"),
        settings: can(me.role, "settings:read"),
      }}
    >
      {children}
    </DashboardShell>
  );
}
