"use server";

import { requireAction } from "@/lib/auth/guard";
import { attempt, DomainError } from "@/lib/errors";
import {
  banPlayer,
  kickPlayer,
  liftTimeout,
  setPlayerAccess,
  setPlayerRoles,
  timeoutPlayer,
  unbanPlayer,
} from "@/lib/players";
import { createReport, type ReportCategory } from "@/lib/reports/store";

export async function kickAction(username: string, reason: string) {
  return attempt(async () => {
    const me = await requireAction("players:moderate");
    await kickPlayer(me, username, reason || null);
  });
}

export async function timeoutAction(username: string, minutes: number, reason: string) {
  return attempt(async () => {
    const me = await requireAction("players:moderate");
    if (!Number.isInteger(minutes) || minutes < 1 || minutes > 525_600)
      throw new DomainError("invalid_input");
    await timeoutPlayer(me, username, { minutes }, reason);
  });
}

export async function banAction(username: string, reason: string) {
  return attempt(async () => {
    const me = await requireAction("players:moderate");
    await banPlayer(me, username, reason);
  });
}

export async function unbanAction(username: string, reason: string) {
  return attempt(async () => {
    const me = await requireAction("players:moderate");
    await unbanPlayer(me, username, reason || null);
  });
}

export async function liftTimeoutAction(username: string) {
  return attempt(async () => {
    const me = await requireAction("players:moderate");
    await liftTimeout(me, username);
  });
}

export async function accessAction(
  userId: string,
  status: "active" | "disabled" | "banned" | undefined,
  approvalStatus: "pending" | "approved" | "rejected" | undefined,
  reason: string,
) {
  return attempt(async () => {
    const me = await requireAction("players:access");
    await setPlayerAccess(me, userId, { status, approvalStatus }, reason);
  });
}

export async function rolesAction(userId: string, roles: string[], reason: string) {
  return attempt(async () => {
    const me = await requireAction("players:roles");
    await setPlayerRoles(me, userId, roles, reason);
  });
}

export async function fileReportAction(
  target: { userId: string | null; name: string },
  category: ReportCategory,
  description: string,
) {
  return attempt(async () => {
    const me = await requireAction("reports:write");
    const { report } = createReport(me, {
      source: "dashboard",
      reporter: { name: me.globalName || me.username },
      target,
      category,
      description,
    });
    return report.id;
  });
}
