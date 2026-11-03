"use client";

import { Button } from "@astryxdesign/core/Button";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { RadioList, RadioListItem } from "@astryxdesign/core/RadioList";
import { Stack } from "@astryxdesign/core/Stack";
import { TextArea } from "@astryxdesign/core/TextArea";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { useAction } from "@/components/useAction";
import { resolveAction } from "../actions";

type Sanction = "none" | "timeout_1h" | "timeout_1d" | "timeout_7d" | "ban";
const MINUTES = { timeout_1h: 60, timeout_1d: 1440, timeout_7d: 10_080 } as const;

/** Resolving can act on the player in the same step; dismissing never does. */
export function ResolveDialog({
  outcome,
  reportId,
  targetName,
  canModerate,
  reasons,
  onClose,
}: {
  outcome: "resolved" | "dismissed" | null;
  reportId: number;
  targetName: string;
  canModerate: boolean;
  reasons: string[];
  onClose: () => void;
}) {
  const t = useTranslations("reports");
  const { run } = useAction();
  const [resolution, setResolution] = useState("");
  const [sanction, setSanction] = useState<Sanction>("none");
  const [busy, setBusy] = useState(false);

  async function submit() {
    if (!outcome) return;
    setBusy(true);
    const s =
      outcome === "dismissed" || sanction === "none"
        ? ({ kind: "none" } as const)
        : sanction === "ban"
          ? ({ kind: "ban" } as const)
          : ({ kind: "timeout", minutes: MINUTES[sanction] } as const);
    const ok = await run(
      () => resolveAction(reportId, outcome, resolution, s),
      t(outcome === "resolved" ? "resolvedToast" : "dismissedToast"),
    );
    setBusy(false);
    if (ok !== false) {
      setResolution("");
      setSanction("none");
      onClose();
    }
  }

  return (
    <Dialog
      isOpen={outcome !== null}
      onOpenChange={(open) => !open && onClose()}
      purpose="form"
      width={520}
    >
      {outcome && (
        <Stack gap={4}>
          <DialogHeader
            title={t(outcome === "resolved" ? "resolveTitle" : "dismissTitle", { id: reportId })}
            onOpenChange={() => onClose()}
          />
          <Stack gap={4} paddingInline={4}>
            {outcome === "resolved" && canModerate && (
              <RadioList
                label={t("sanction", { name: targetName })}
                value={sanction}
                onChange={(v: string) => setSanction(v as Sanction)}
              >
                {(["none", "timeout_1h", "timeout_1d", "timeout_7d", "ban"] as const).map((s) => (
                  <RadioListItem key={s} value={s} label={t(`sanctions.${s}`)} />
                ))}
              </RadioList>
            )}
            <TextArea
              label={t(outcome === "resolved" ? "resolution" : "dismissedBecause")}
              description={sanction !== "none" ? t("resolutionShown") : undefined}
              value={resolution}
              onChange={setResolution}
              rows={3}
              maxLength={400}
              isRequired
              placeholder={reasons[0]}
            />
          </Stack>
          <Stack direction="horizontal" gap={2} justify="end" paddingInline={4} paddingBlockEnd={4}>
            <Button label={t("cancel")} variant="ghost" onClick={onClose} />
            <Button
              label={t(outcome === "resolved" ? "resolve" : "dismiss")}
              variant={sanction === "ban" ? "destructive" : "primary"}
              isDisabled={resolution.trim().length < 3}
              isLoading={busy}
              onClick={submit}
            />
          </Stack>
        </Stack>
      )}
    </Dialog>
  );
}
