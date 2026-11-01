import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { config } from "@/lib/config";
import { fleet, totals } from "@/lib/fleet/status";
import { ControllersClient } from "./ControllersClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("fleet"))("title") };
}

export default async function ControllersPage({
  searchParams,
}: {
  searchParams: Promise<{ state?: string; q?: string; region?: string }>;
}) {
  const me = await requirePage("controllers:read");
  const sp = await searchParams;
  const views = fleet();
  return (
    <ControllersClient
      controllers={views}
      totals={totals(views)}
      filters={{ state: sp.state ?? "", q: sp.q ?? "", region: sp.region ?? "" }}
      canWrite={can(me.role, "controllers:write")}
      dashboardUrl={config.baseUrl}
      insecure={!config.baseUrl.startsWith("https:")}
    />
  );
}
