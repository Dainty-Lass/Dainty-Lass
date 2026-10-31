"use server";

import { recordAudit } from "@/lib/audit/record";
import { requireAction } from "@/lib/auth/guard";
import { pruneChat } from "@/lib/chat/store";
import { attempt } from "@/lib/errors";
import { getSetting, type Settings } from "@/lib/settings";
import { updateSettings } from "@/lib/settings-change";

export async function saveSettingsAction(patch: Partial<Settings>) {
  return attempt(async () => {
    updateSettings(await requireAction("settings:write"), patch);
  });
}

export async function pruneChatAction() {
  return attempt(async () => {
    const me = await requireAction("settings:write");
    const days = getSetting("chatRetentionDays");
    const removed = pruneChat(days);
    recordAudit(me, {
      action: "chat.prune",
      targetType: "settings",
      targetId: "chatRetentionDays",
      targetLabel: String(days),
      after: { removed },
    });
    return removed;
  });
}
