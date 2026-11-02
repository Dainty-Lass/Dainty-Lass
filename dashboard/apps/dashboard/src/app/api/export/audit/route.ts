import { NextResponse } from "next/server";
import { queryAudit } from "@/lib/audit/query";
import { can } from "@/lib/auth/permissions";
import { currentStaff } from "@/lib/auth/session";
import { csv, csvResponse } from "@/lib/csv";
import { currentScope, scopeIds } from "@/lib/fleet/scope";
import { auditQueryFromParams } from "@/lib/query-params";

export async function GET(request: Request) {
  const me = await currentStaff();
  if (!me || !can(me.role, "audit:read"))
    return NextResponse.json({ error: "forbidden" }, { status: 403 });
  const q = auditQueryFromParams(new URL(request.url).searchParams);
  const { events } = queryAudit({
    ...q,
    controllerIds: scopeIds(await currentScope()),
    page: 1,
    pageSize: 500,
  });
  return csvResponse(
    `audit-${new Date().toISOString().slice(0, 10)}.csv`,
    csv(
      [
        "id",
        "time_utc",
        "controller",
        "actor",
        "action",
        "target_type",
        "target_id",
        "target",
        "reason",
        "before",
        "after",
        "reverted_by",
        "revert_of",
      ],
      events.map((e) => [
        e.id,
        new Date(e.at).toISOString(),
        e.controllerId,
        e.actorName,
        e.action,
        e.targetType,
        e.targetId,
        e.targetLabel,
        e.reason,
        e.before,
        e.after,
        e.revertedById,
        e.revertOfId,
      ]),
    ),
  );
}
