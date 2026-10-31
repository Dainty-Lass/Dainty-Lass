"use server";

import { requireAction } from "@/lib/auth/guard";
import { attempt, DomainError } from "@/lib/errors";
import {
  banEverywhere,
  banPlayer,
  kickPlayer,
  liftTimeout,
  setPlayerAccess,
  setPlayerRoles,
  timeoutPlayer,
  unbanPlayer,
} from "@/lib/players";
import { createReport, type ReportCategory } from "@/lib/reports/store";

export async function kickAction(controllerId: string, username: string, reason: string) {
  return attempt(async () => {
    const me = await requireAction("players:moderate");
    await kickPlayer(me, controllerId, username, reason || null);
  });
}

export async function timeoutAction(
  controllerId: string,
  username: string,
  minutes: number,
  reason: string,
) {
  return attempt(async () => {
    const me = await requireAction("players:moderate");
    if (!Number.isInteger(minutes) || minutes < 1 || minutes > 525_600)
      throw new DomainError("invalid_input");
    await timeoutPlayer(me, controllerId, username, { minutes }, reason);
  });
}

export async function banAction(controllerId: string, username: string, reason: string) {
  return attempt(async () => {
    const me = await requireAction("players:moderate");
    await banPlayer(me, controllerId, username, reason);
  });
}

/** The same ban on every reachable controller; the answer says what happened on each. */
export async function banEverywhereAction(username: string, reason: string) {
  return attempt(async () => banEverywhere(await requireAction("players:fleet"), username, reason));
}

export async function unbanAction(controllerId: string, username: string, reason: string) {
  return attempt(async () => {
    const me = await requireAction("players:moderate");
    await unbanPlayer(me, controllerId, username, reason || null);
  });
}

export async function liftTimeoutAction(controllerId: string, username: string) {
  return attempt(async () => {
    const me = await requireAction("players:moderate");
    await liftTimeout(me, controllerId, username);
  });
}

export async function accessAction(
  controllerId: string,
  userId: string,
  status: "active" | "disabled" | "banned" | undefined,
  approvalStatus: "pending" | "approved" | "rejected" | undefined,
  reason: string,
) {
  return attempt(async () => {
    const me = await requireAction("players:access");
    await setPlayerAccess(me, controllerId, userId, { status, approvalStatus }, reason);
  });
}

export async function rolesAction(
  controllerId: string,
  userId: string,
  roles: string[],
  reason: string,
) {
  return attempt(async () => {
    const me = await requireAction("players:roles");
    await setPlayerRoles(me, controllerId, userId, roles, reason);
  });
}

export async function fileReportAction(
  controllerId: string,
  target: { userId: string | null; name: string },
  category: ReportCategory,
  description: string,
) {
  return attempt(async () => {
    const me = await requireAction("reports:write");
    const { report } = createReport(me, {
      controllerId,
      source: "dashboard",
      reporter: { name: me.globalName || me.username },
      target,
      category,
      description,
    });
    return report.id;
  });
}
