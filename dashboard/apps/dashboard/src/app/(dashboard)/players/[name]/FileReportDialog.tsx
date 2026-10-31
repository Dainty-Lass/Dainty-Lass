"use client";

import { Button } from "@astryxdesign/core/Button";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { Selector } from "@astryxdesign/core/Selector";
import { Stack } from "@astryxdesign/core/Stack";
import { TextArea } from "@astryxdesign/core/TextArea";
import { useRouter } from "next/navigation";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { useAction } from "@/components/useAction";
import { REPORT_CATEGORIES, type ReportCategory } from "@/lib/reports/categories";
import { fileReportAction } from "../actions";

/** A report staff file themselves, from something seen on Discord or in game: it joins the same queue. */
export function FileReportDialog({
  isOpen,
  controllerId,
  target,
  onClose,
}: {
  isOpen: boolean;
  controllerId: string;
  target: { userId: string | null; name: string };
  onClose: () => void;
}) {
  const t = useTranslations("reports");
  const router = useRouter();
  const { run } = useAction();
  const [category, setCategory] = useState<ReportCategory>("harassment");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);

  return (
    <Dialog isOpen={isOpen} onOpenChange={(open) => !open && onClose()} purpose="form" width={480}>
      <Stack gap={4}>
        <DialogHeader
          title={t("fileTitle", { name: target.name })}
          onOpenChange={() => onClose()}
        />
        <Stack gap={4} paddingInline={4}>
          <Selector
            label={t("columns.category")}
            value={category}
            onChange={(v) => setCategory(v as ReportCategory)}
            options={REPORT_CATEGORIES.map((c) => ({ value: c, label: t(`categories.${c}`) }))}
            width="100%"
          />
          <TextArea
            label={t("columns.description")}
            value={description}
            onChange={setDescription}
            rows={5}
            maxLength={2000}
            isRequired
          />
        </Stack>
        <Stack direction="horizontal" gap={2} justify="end" paddingInline={4} paddingBlockEnd={4}>
          <Button label={t("cancel")} variant="ghost" onClick={onClose} />
          <Button
            label={t("file")}
            variant="primary"
            isLoading={busy}
            isDisabled={description.trim().length < 3}
            onClick={async () => {
              setBusy(true);
              const id = await run(
                () => fileReportAction(controllerId, target, category, description),
                t("filed"),
              );
              setBusy(false);
              if (id !== false) {
                setDescription("");
                onClose();
                router.push(`/reports/${id}`);
              }
            }}
          />
        </Stack>
      </Stack>
    </Dialog>
  );
}
