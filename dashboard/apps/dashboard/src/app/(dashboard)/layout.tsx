import type { ReactNode } from "react";
import { avatarUrl } from "@/lib/auth/discord";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { reportCounts } from "@/lib/reports/store";
import { getSettings } from "@/lib/settings";
import { DashboardShell } from "./DashboardShell";

export default async function DashboardLayout({ children }: { children: ReactNode }) {
  const me = await requirePage("overview:read");
  const counts = reportCounts();
  const { serverName } = getSettings();
  return (
    <DashboardShell
      serverName={serverName}
      me={{
        name: me.globalName || me.username,
        role: me.role,
        avatar: avatarUrl(me.discordId, me.avatar),
      }}
      openReports={counts.open}
      nav={{
        filter: can(me.role, "chat:read"),
        staff: can(me.role, "staff:read"),
        settings: can(me.role, "settings:read"),
      }}
    >
      {children}
    </DashboardShell>
  );
}
