"use server";

import { requireAction } from "@/lib/auth/guard";
import { isRole, type Role } from "@/lib/auth/permissions";
import { attempt, DomainError } from "@/lib/errors";
import { addStaff, updateStaff } from "@/lib/staff";

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
  change: { role?: Role; disabled?: boolean; note?: string | null },
) {
  return attempt(async () => {
    const me = await requireAction("staff:write");
    if (change.role !== undefined && !isRole(change.role)) throw new DomainError("invalid_input");
    updateStaff(me, id, change);
  });
}
