import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { getTranslations } from "next-intl/server";
import { requirePage } from "@/lib/auth/guard";
import { getCrashReport } from "@/lib/crash-reports/store";
import { CrashReportDetailClient } from "./CrashReportDetailClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("crashReports"))("title") };
}

export default async function CrashReportPage({ params }: { params: Promise<{ id: string }> }) {
  await requirePage("reports:read");
  const report = getCrashReport((await params).id);
  if (!report) notFound();
  return <CrashReportDetailClient report={report} />;
}
