"use server";

import { requireAction } from "@/lib/auth/guard";
import { attempt, DomainError } from "@/lib/errors";
import { banPlayer, timeoutPlayer } from "@/lib/players";
import { REPORT_PRIORITIES, type ReportPriority } from "@/lib/reports/categories";
import {
  addNote,
  claimReport,
  closeReport,
  getReport,
  reopenReport,
  updateReport,
} from "@/lib/reports/store";

export async function claimAction(id: number) {
  return attempt(async () => {
    claimReport(await requireAction("reports:write"), id);
  });
}

export async function releaseAction(id: number) {
  return attempt(async () => {
    const me = await requireAction("reports:write");
    updateReport(me, id, { status: "open", assigneeId: null });
  });
}

export async function reopenAction(id: number) {
  return attempt(async () => {
    reopenReport(await requireAction("reports:write"), id);
  });
}

export async function priorityAction(id: number, priority: ReportPriority) {
  return attempt(async () => {
    if (!REPORT_PRIORITIES.includes(priority)) throw new DomainError("invalid_input");
    updateReport(await requireAction("reports:write"), id, { priority });
  });
}

export async function noteAction(id: number, body: string) {
  return attempt(async () => {
    addNote(await requireAction("reports:write"), id, body);
  });
}

/**
 * Closes a report, optionally acting on the player in the same step. The sanction runs first: if the
 * game refuses it, the report stays open rather than claiming an action that did not happen.
 */
export async function resolveAction(
  id: number,
  outcome: "resolved" | "dismissed",
  resolution: string,
  sanction: { kind: "none" } | { kind: "timeout"; minutes: number } | { kind: "ban" },
) {
  return attempt(async () => {
    const me = await requireAction("reports:write");
    const report = getReport(id);
    if (!report) throw new DomainError("not_found");
    if (outcome === "resolved" && sanction.kind !== "none") {
      await requireAction("players:moderate");
      const reason = resolution.trim() || `Report #${id}`;
      if (sanction.kind === "ban") await banPlayer(me, report.targetName, reason);
      else await timeoutPlayer(me, report.targetName, { minutes: sanction.minutes }, reason);
    }
    closeReport(me, id, outcome, resolution);
  });
}
