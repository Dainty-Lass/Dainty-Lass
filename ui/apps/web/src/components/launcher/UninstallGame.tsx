"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { Spinner } from "@astryxdesign/core/Spinner";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import type { LauncherError, UninstallPlan } from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { Trash2 } from "lucide-react";
import { useFormatter, useTranslations } from "next-intl";
import { useState } from "react";
import { toast } from "sonner";

/** Uninstalls an installed version after a dialog naming its folder and the space freed. */
export function UninstallGame({
  version,
  name,
  hideFolder,
  onDone,
}: {
  version: string;
  name: string;
  /** Streamer mode: the folder's words in its place. */
  hideFolder: string | null;
  onDone: () => void;
}) {
  const t = useTranslations("launcher.uninstall");
  const te = useTranslations("errors");
  const format = useFormatter();
  const { api } = useBridge();
  const [open, setOpen] = useState(false);
  const [plan, setPlan] = useState<UninstallPlan | null>(null);
  const [working, setWorking] = useState(false);

  // A file that would not go comes back as `unknown` with its name
  const refuse = (error: LauncherError) =>
    toast.error(
      error.code === "unknown" && error.detail
        ? t("failed", { file: error.detail })
        : te(error.code, { detail: error.detail ?? "" }),
    );

  const start = async () => {
    if (!api) return;
    setPlan(null);
    setOpen(true);
    const outcome = await api.gameUninstallPlan(version);
    if (outcome.ok) return setPlan(outcome.plan);
    setOpen(false);
    refuse(outcome.error);
  };

  const confirm = async () => {
    if (!api) return;
    setWorking(true);
    try {
      const r = await api.gameUninstall(version);
      if (!r.ok) return refuse(r.error);
      toast.success(t("done", { version: name }));
      setOpen(false);
      onDone();
    } finally {
      setWorking(false);
    }
  };

  const size = plan ? format.number(plan.bytes / 1e9, { maximumFractionDigits: 1 }) : "";
  const folder = plan ? (hideFolder ?? plan.folder) : "";

  return (
    <>
      <Button
        label={t("action")}
        size="sm"
        variant="ghost"
        icon={<Trash2 size={16} />}
        onClick={() => void start()}
      />
      <Dialog
        isOpen={open}
        onOpenChange={(next) => !working && setOpen(next)}
        purpose="form"
        width={480}
      >
        <Stack gap={4}>
          <DialogHeader
            title={t("title", { version: name })}
            onOpenChange={(next) => !working && setOpen(next)}
          />
          <Stack gap={3} paddingInline={4}>
            {plan === null ? (
              <Stack direction="horizontal" gap={2} align="center">
                <Spinner size="sm" />
                <Text type="supporting">{t("planning")}</Text>
              </Stack>
            ) : (
              <>
                <Text>{t(plan.wholeFolder ? "whole" : "listed", { folder, size })}</Text>
                <Banner status="warning" title={t("permanent")} collapsible={false} />
              </>
            )}
          </Stack>
          <Stack direction="horizontal" gap={2} justify="end" paddingInline={4} paddingBlockEnd={4}>
            <Button
              label={t("keep")}
              variant="ghost"
              isDisabled={working}
              onClick={() => setOpen(false)}
            />
            <Button
              label={t("confirm")}
              variant="destructive"
              isDisabled={plan === null}
              isLoading={working}
              onClick={() => void confirm()}
            />
          </Stack>
        </Stack>
      </Dialog>
    </>
  );
}
