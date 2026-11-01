"use server";

import { requireAction } from "@/lib/auth/guard";
import { isRole, type Role } from "@/lib/auth/permissions";
import { attempt, DomainError } from "@/lib/errors";
import { addStaff, type MemberChange, updateStaff } from "@/lib/staff";

export async function addStaffAction(
  discordId: string,
  username: string,
  role: Role,
  note: string,
) {
  return attempt(async () => {
    const me = await requireAction("staff:write");
    if (!isRole(role)) throw new DomainError("invalid_input");
    addStaff(me, { discordId: discordId.trim(), username, role, note });
  });
}

export async function updateStaffAction(
  id: number,
  change: Pick<MemberChange, "role" | "roleSource" | "disabled" | "note" | "approvedAt">,
) {
  return attempt(async () => {
    const me = await requireAction("staff:write");
    const { role, roleSource, disabled, note, approvedAt } = change;
    if (role !== undefined && !isRole(role)) throw new DomainError("invalid_input");
    if (roleSource !== undefined && roleSource !== "manual" && roleSource !== "guild")
      throw new DomainError("invalid_input");
    // Approval here is always "now, by hand" or "not approved"; the client does not pick the time.
    const approval =
      approvedAt === undefined
        ? {}
        : approvedAt === null
          ? { approvedAt: null, approvedVia: null }
          : { approvedAt: Date.now(), approvedVia: "manual" as const };
    updateStaff(me, id, {
      ...(role !== undefined ? { role } : {}),
      ...(roleSource !== undefined ? { roleSource } : {}),
      ...(disabled !== undefined ? { disabled: Boolean(disabled) } : {}),
      ...(note !== undefined ? { note } : {}),
      ...approval,
    });
  });
}
