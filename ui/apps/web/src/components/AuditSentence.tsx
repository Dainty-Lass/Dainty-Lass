"use client";

import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { useTranslations } from "next-intl";
import type { AuditEvent } from "@/lib/db/schema";

/** "Moonwick banned Grevel": the catalog holds each action's sentence, with {actor} and {target}. */
export function useAuditText() {
  // Keys are composed from the action, so the typed signature cannot help; t.has() guards instead.
  const t = useTranslations("audit.actions") as unknown as {
    (key: string, values?: Record<string, string>): string;
    has(key: string): boolean;
  };
  return (ev: Pick<AuditEvent, "action" | "actorName" | "targetLabel" | "targetId">) => {
    const values = { actor: ev.actorName, target: ev.targetLabel ?? ev.targetId ?? "—" };
    const key = ev.action.replaceAll(".", "_");
    return t.has(key) ? t(key, values) : t("unknown", { ...values, action: ev.action });
  };
}

export function AuditSentence({
  event,
  withTime = true,
}: {
  event: AuditEvent;
  withTime?: boolean;
}) {
  const text = useAuditText();
  return (
    <Stack direction="horizontal" gap={2} justify="between" align="start">
      <Text maxLines={2}>{text(event)}</Text>
      {withTime && <Timestamp value={new Date(event.at).toISOString()} format="relative_short" />}
    </Stack>
  );
}
