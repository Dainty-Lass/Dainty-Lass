"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { NumberInput } from "@astryxdesign/core/NumberInput";
import { SegmentedControl, SegmentedControlItem } from "@astryxdesign/core/SegmentedControl";
import { Stack } from "@astryxdesign/core/Stack";
import { Switch } from "@astryxdesign/core/Switch";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";
import { ReasonField } from "@/components/ReasonField";
import { useAction } from "@/components/useAction";
import type { FleetOutcome } from "@/lib/players";
import { banAction, banEverywhereAction, kickAction, timeoutAction, unbanAction } from "../actions";

export type ModerationKind = "kick" | "timeout" | "ban" | "unban";

const DURATIONS = { "15m": 15, "1h": 60, "1d": 1440, "7d": 10_080, "30d": 43_200 } as const;

export function ModerationDialog({
  kind,
  controllerId,
  canFleet,
  username,
  reasons,
  onClose,
}: {
  kind: ModerationKind | null;
  controllerId: string;
  /** Offer one ban on every controller; for admins with more than one controller. */
  canFleet: boolean;
  username: string;
  reasons: string[];
  onClose: () => void;
}) {
  const t = useTranslations("moderation");
  const { run } = useAction();
  const [reason, setReason] = useState("");
  const [duration, setDuration] = useState<keyof typeof DURATIONS | "custom">("1h");
  const [custom, setCustom] = useState<number>(120);
  const [busy, setBusy] = useState(false);
  const [everywhere, setEverywhere] = useState(false);

  const minutes = duration === "custom" ? custom : DURATIONS[duration];
  const optional = kind === "kick" || kind === "unban";
  const ready = optional || reason.trim().length >= 3;

  async function submit() {
    if (!kind) return;
    setBusy(true);
    if (kind === "ban" && everywhere) {
      const outcomes = (await run(() => banEverywhereAction(username, reason))) as
        | FleetOutcome[]
        | false;
      setBusy(false);
      if (outcomes === false) return;
      const done = outcomes.filter((o) => o.result === "done").length;
      const failed = outcomes.filter((o) => o.result === "failed" || o.result === "skipped");
      toast[failed.length ? "warning" : "success"](
        t("fleetDone", {
          done,
          total: outcomes.length,
          absent: outcomes.filter((o) => o.result === "absent").length,
          missed: failed.length,
        }),
        {
          description: failed.length ? failed.map((o) => o.name).join(", ") : undefined,
        },
      );
      setReason("");
      setEverywhere(false);
      onClose();
      return;
    }
    const done = await run(
      () =>
        kind === "kick"
          ? kickAction(controllerId, username, reason)
          : kind === "timeout"
            ? timeoutAction(controllerId, username, minutes, reason)
            : kind === "ban"
              ? banAction(controllerId, username, reason)
              : unbanAction(controllerId, username, reason),
      t(`done.${kind}`, { name: username }),
    );
    setBusy(false);
    if (done !== false) {
      setReason("");
      onClose();
    }
  }

  return (
    <Dialog
      isOpen={kind !== null}
      onOpenChange={(open) => !open && onClose()}
      purpose="form"
      width={480}
    >
      {kind && (
        <Stack gap={4}>
          <DialogHeader
            title={t(`titles.${kind}`, { name: username })}
            subtitle={t(`explain.${kind}`)}
            onOpenChange={() => onClose()}
          />
          <Stack gap={4} paddingInline={4}>
            {kind === "ban" && (
              <Banner status="warning" title={t("banWarning")} collapsible={false} />
            )}
            {kind === "ban" && canFleet && (
              <Switch
                label={t("banEverywhere")}
                description={t("banEverywhereHint")}
                value={everywhere}
                onChange={setEverywhere}
              />
            )}
            {kind === "timeout" && (
              <Stack gap={2}>
                <SegmentedControl
                  label={t("duration")}
                  value={duration}
                  onChange={(v) => setDuration(v as typeof duration)}
                  layout="fill"
                >
                  {Object.keys(DURATIONS).map((d) => (
                    <SegmentedControlItem
                      key={d}
                      value={d}
                      label={t(`durations.${d as keyof typeof DURATIONS}`)}
                    />
                  ))}
                  <SegmentedControlItem value="custom" label={t("durations.custom")} />
                </SegmentedControl>
                {duration === "custom" && (
                  <NumberInput
                    label={t("minutes")}
                    value={custom}
                    onChange={(v) => setCustom(Math.max(1, Math.min(525_600, Math.round(v))))}
                  />
                )}
              </Stack>
            )}
            <ReasonField
              label={t("reason")}
              presets={reasons}
              value={reason}
              onChange={setReason}
              isOptional={optional}
            />
          </Stack>
          <Stack direction="horizontal" gap={2} justify="end" paddingInline={4} paddingBlockEnd={4}>
            <Button label={t("cancel")} variant="ghost" onClick={onClose} />
            <Button
              label={t(`confirm.${kind}`)}
              variant={kind === "unban" ? "primary" : "destructive"}
              isDisabled={!ready}
              isLoading={busy}
              onClick={submit}
            />
          </Stack>
        </Stack>
      )}
    </Dialog>
  );
}
