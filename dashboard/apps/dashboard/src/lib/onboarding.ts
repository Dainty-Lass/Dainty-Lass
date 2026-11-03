import { eq } from "drizzle-orm";
import { recordAudit } from "@/lib/audit/record";
import { addGuildRole } from "@/lib/auth/discord";
import { rank } from "@/lib/auth/permissions";
import { db, schema, tx } from "@/lib/db";
import type { Staff } from "@/lib/db/schema";
import { DomainError } from "@/lib/errors";
import { getSettings, type Settings } from "@/lib/settings";

/**
 * Approval, when the server asks for it (Settings → Onboarding). Someone is approved when they hold the
 * guild's approved role, when an admin approved them, or when they accepted the current terms on the
 * welcome page. Moderators and above never wait on it: whoever made them staff has vouched for them.
 */
export function isApproved(me: Staff, s: Settings = getSettings()): boolean {
  if (!me.approvedAt) return false;
  // Accepting is per version: new terms ask again. A guild role or an admin's say-so does not lapse.
  return me.approvedVia !== "tos" || (me.tosVersion ?? 0) >= s.tosVersion;
}

export function needsOnboarding(me: Staff, s: Settings = getSettings()): boolean {
  return s.requireApproval && rank(me.role) < rank("moderator") && !isApproved(me, s);
}

const NAME = /^[\p{L}\p{N} _.'-]{2,32}$/u;
// The launcher's rule for player names (dainty-lass docs/admin-api.md).
const GAME_NAME = /^\w{3,16}$/;

export interface OnboardingResult {
  /** Whether Discord now shows the approved role: null when the server has none set up to add. */
  roleAdded: boolean | null;
}

/**
 * The welcome page's account: a display name, optionally the in-game name, and the terms accepted at the
 * version the person read. The approval is recorded here whatever Discord says; adding the guild role
 * comes after, so a missing bot permission costs the role, not the account.
 */
export async function completeOnboarding(
  me: Staff,
  input: { displayName: string; gameName: string; acceptedVersion: number },
): Promise<OnboardingResult> {
  const s = getSettings();
  const displayName = input.displayName.trim().replace(/\s+/g, " ");
  const gameName = input.gameName.trim();
  if (!NAME.test(displayName)) throw new DomainError("invalid_input", "displayName");
  if (gameName && !GAME_NAME.test(gameName)) throw new DomainError("invalid_input", "gameName");
  // Accepting a page loaded before the terms changed would accept terms never shown.
  if (input.acceptedVersion !== s.tosVersion) throw new DomainError("conflict");

  tx((t) => {
    t.update(schema.staff)
      .set({
        displayName,
        gameName: gameName || null,
        approvedAt: Date.now(),
        approvedVia: "tos",
        tosVersion: s.tosVersion,
      })
      .where(eq(schema.staff.id, me.id))
      .run();
    recordAudit(
      me,
      {
        action: "member.onboard",
        targetType: "staff",
        targetId: me.id,
        targetLabel: displayName,
        after: { displayName, gameName: gameName || null, tosVersion: s.tosVersion },
      },
      t,
    );
  });

  // Without Discord linked there is nobody in the guild to give the role to.
  if (!s.discordGuildId || !s.discordApprovedRoleId || !me.discordId) return { roleAdded: null };
  const roleAdded = await addGuildRole(s.discordGuildId, me.discordId, s.discordApprovedRoleId);
  recordAudit(me, {
    action: roleAdded ? "member.role_added" : "member.role_failed",
    targetType: "staff",
    targetId: me.id,
    targetLabel: displayName,
    after: { roleId: s.discordApprovedRoleId },
  });
  return { roleAdded };
}

/** How many accounts wait on approval, for the Members badge. */
export function pendingCount(s: Settings = getSettings()): number {
  if (!s.requireApproval) return 0;
  return db
    .select()
    .from(schema.staff)
    .all()
    .filter((m) => !m.disabled && needsOnboarding(m, s)).length;
}
