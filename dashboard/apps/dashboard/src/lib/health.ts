import type { Hunt, ServiceState } from "@dainty/fleet";
import { desc, inArray } from "drizzle-orm";
import { config, discordConfigured } from "@/lib/config";
import { db, schema } from "@/lib/db";
import { type ControllerView, type FleetTotals, fleet, history, totals } from "@/lib/fleet/status";

export type { ServiceState } from "@dainty/fleet";

export type FleetHunt = Hunt & { controllerId: string; controllerName: string };

export interface Overview {
  controllers: ControllerView[];
  totals: FleetTotals;
  hunts: FleetHunt[];
  history: { at: number; online: number; hunts: number }[];
  discord: ServiceState;
  ingest: ServiceState;
  lastChatAt: number | null;
}

/**
 * The overview for a set of controllers, all from what they last reported: no call leaves the dashboard
 * while the page renders, however many controllers there are.
 */
export function overview(controllerIds?: string[]): Overview {
  const all = fleet();
  const views = controllerIds ? all.filter((v) => controllerIds.includes(v.id)) : all;
  const hunts = views
    .filter((v) => v.state === "online" || v.state === "stale")
    .flatMap((v) =>
      (v.status?.hunts?.hunts ?? []).map((h) => ({
        ...h,
        controllerId: v.id,
        controllerName: v.name,
      })),
    )
    .sort((a, b) => (b.startedAt ?? "").localeCompare(a.startedAt ?? ""));
  const last = db
    .select({ at: schema.chatMessages.at })
    .from(schema.chatMessages)
    .where(controllerIds ? inArray(schema.chatMessages.controllerId, controllerIds) : undefined)
    .orderBy(desc(schema.chatMessages.at))
    .limit(1)
    .get();
  return {
    controllers: views,
    totals: totals(views),
    hunts,
    history: history(
      views.map((v) => v.id),
      24,
    ),
    discord: discordConfigured() ? "ok" : "not_configured",
    ingest: config.ingestKey && config.ingestKey.length >= 32 ? "ok" : "not_configured",
    lastChatAt: last?.at ?? null,
  };
}
