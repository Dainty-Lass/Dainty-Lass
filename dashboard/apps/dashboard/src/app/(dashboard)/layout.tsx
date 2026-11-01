import type { ReactNode } from "react";
import { avatarUrl } from "@/lib/auth/discord";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { currentScope, scopeIds } from "@/lib/fleet/scope";
import { fleet } from "@/lib/fleet/status";
import { reportCounts } from "@/lib/reports/store";
import { getSettings } from "@/lib/settings";
import { DashboardShell } from "./DashboardShell";

export default async function DashboardLayout({ children }: { children: ReactNode }) {
  const me = await requirePage("overview:read");
  const scope = await currentScope();
  const counts = reportCounts(scopeIds(scope));
  const { serverName } = getSettings();
  const controllers = fleet();
  return (
    <DashboardShell
      serverName={serverName}
      me={{
        name: me.globalName || me.username,
        role: me.role,
        avatar: avatarUrl(me.discordId, me.avatar),
      }}
      openReports={counts.open}
      scope={scope.kind === "one" ? scope.id : "all"}
      controllers={controllers.map((c) => ({ id: c.id, name: c.name, state: c.state }))}
      nav={{
        filter: can(me.role, "chat:read"),
        controllers: can(me.role, "controllers:read"),
        staff: can(me.role, "staff:read"),
        settings: can(me.role, "settings:read"),
      }}
    >
      {children}
    </DashboardShell>
  );
}
