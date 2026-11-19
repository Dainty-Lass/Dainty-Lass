import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { requirePage } from "@/lib/auth/guard";
import { listCrashReports } from "@/lib/crash-reports/store";
import { currentScope, scopeIds } from "@/lib/fleet/scope";
import { CrashReportsClient } from "./CrashReportsClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("crashReports"))("title") };
}

export default async function CrashReportsPage() {
  await requirePage("reports:read");
  const ids = scopeIds(await currentScope());
  return <CrashReportsClient reports={listCrashReports({ controllerIds: ids })} />;
}
