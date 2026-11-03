import type { Metadata } from "next";
import { getTranslations } from "next-intl/server";
import { requirePage } from "@/lib/auth/guard";
import { can } from "@/lib/auth/permissions";
import { listRevisions } from "@/lib/revisions";
import { getSettings } from "@/lib/settings";
import { listStaff } from "@/lib/staff";
import { StaffClient } from "./StaffClient";

export async function generateMetadata(): Promise<Metadata> {
  return { title: (await getTranslations("staff"))("title") };
}

export default async function StaffPage() {
  const me = await requirePage("staff:read");
  const { discordGuildId, requireApproval } = getSettings();
  return (
    <StaffClient
      staff={listStaff().map((s) => ({
        ...s,
        avatarUrl: s.avatar,
      }))}
      me={{ id: me.id, role: me.role }}
      canWrite={can(me.role, "staff:write")}
      canRestore={can(me.role, "audit:revert")}
      guildSync={Boolean(discordGuildId)}
      requireApproval={requireApproval}
      revisions={listRevisions("staff", 20).map(({ snapshot, ...r }) => r)}
    />
  );
}
