"use client";

import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Text } from "@astryxdesign/core/Text";
import { useTranslations } from "next-intl";
import type { ControllerState as State } from "@/lib/fleet/status";

export const STATE_DOT: Record<State, "success" | "warning" | "error" | "neutral" | "accent"> = {
  online: "success",
  stale: "warning",
  offline: "error",
  pending: "accent",
  disabled: "neutral",
  not_configured: "neutral",
};

/** Down first, so a fleet's problems lead the list. */
export const STATE_ORDER: Record<State, number> = {
  offline: 0,
  stale: 1,
  not_configured: 2,
  pending: 3,
  online: 4,
  disabled: 5,
};

export function ControllerState({ state, withText = true }: { state: State; withText?: boolean }) {
  const t = useTranslations("fleet.states");
  return (
    <Stack direction="horizontal" gap={1.5} align="center">
      <StatusDot variant={STATE_DOT[state]} label={t(state)} isPulsing={state === "online"} />
      {withText && <Text type="supporting">{t(state)}</Text>}
    </Stack>
  );
}
