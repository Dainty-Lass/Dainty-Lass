import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { auditActors, queryAudit } from "@/lib/audit/query";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { currentScope, scopeIds } from "@/lib/fleet/scope";
import { auditQueryFromParams, type SearchParams } from "@/lib/query-params";
import { AuditClient } from "./AuditClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("audit"))("title") };
}

export default async function AuditPage({ searchParams }: { searchParams: Promise<SearchParams> }) {
  const me = await requirePage("audit:read");
  const raw = await searchParams;
  const q = auditQueryFromParams(raw);
  const { events, total } = queryAudit({
    ...q,
    controllerIds: scopeIds(await currentScope()),
    pageSize: 50,
  });
  const exportQuery = new URLSearchParams(
    Object.entries(raw).flatMap(([k, v]) =>
      typeof v === "string" && k !== "page" ? [[k, v]] : [],
    ),
  ).toString();
  return (
    <AuditClient
      events={events}
      total={total}
      page={q.page ?? 1}
      actors={auditActors()}
      filters={{
        actor: q.actorId ? String(q.actorId) : "",
        target: q.targetType ?? "",
        action: q.action ?? "",
        q: q.text ?? "",
        range: q.range ?? "",
        revertible: Boolean(q.revertibleOnly),
      }}
      canRevert={can(me.role, "audit:revert")}
      exportHref={`/api/export/audit${exportQuery ? `?${exportQuery}` : ""}`}
    />
  );
}
