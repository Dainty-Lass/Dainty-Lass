"use client";

import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Stack } from "@astryxdesign/core/Stack";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { ReasonField } from "@/components/ReasonField";
import { useAction } from "@/components/useAction";
import type { BackendAccount, SaveSnapshot } from "@/lib/game/backend";
import { restoreSaveAction, saveSnapshotsAction } from "../actions";

/** The saves a player's in-game /reset and staff restores set aside (kept 30 days), and putting one back. */
export function SavesDialog({
  isOpen,
  controllerId,
  account,
  online,
  reasons,
  onClose,
}: {
  isOpen: boolean;
  controllerId: string;
  account: BackendAccount;
  online: boolean;
  reasons: string[];
  onClose: () => void;
}) {
  const t = useTranslations("saves");
  const te = useTranslations("errors");
  const { run } = useAction();
  const [snapshots, setSnapshots] = useState<SaveSnapshot[] | null>(null);
  const [picked, setPicked] = useState<SaveSnapshot | null>(null);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!isOpen) return;
    setSnapshots(null);
    setPicked(null);
    setReason("");
    let live = true;
    void saveSnapshotsAction(controllerId, account.userId).then((found) => {
      if (!live) return;
      if (!found.ok) toast.error(te(found.code, { detail: found.detail ?? "" }));
      setSnapshots(found.ok ? (found.value ?? []) : []);
    });
    return () => {
      live = false;
    };
  }, [isOpen, controllerId, account.userId, te]);

  async function restore() {
    if (!picked) return;
    setBusy(true);
    const ok = await run(
      () => restoreSaveAction(controllerId, account.userId, picked.id, reason),
      t("restored", { name: account.displayName }),
    );
    setBusy(false);
    if (ok !== false) onClose();
  }

  return (
    <Dialog isOpen={isOpen} onOpenChange={(open) => !open && onClose()} purpose="form" width={720}>
      <Stack gap={4}>
        <DialogHeader
          title={
            picked
              ? t("confirmTitle", { name: account.displayName })
              : t("title", { name: account.displayName })
          }
          subtitle={picked ? t("confirmExplain") : t("explain")}
          onOpenChange={() => onClose()}
        />
        {picked ? (
          <Stack gap={4} paddingInline={4}>
            <Text>
              {t("confirmWhich")} <Timestamp value={picked.createdAt} format="date_time" />
            </Text>
            {online && <Text color="secondary">{t("whenOnline")}</Text>}
            <ReasonField
              label={t("reason")}
              presets={reasons}
              value={reason}
              onChange={setReason}
            />
          </Stack>
        ) : snapshots === null ? (
          <Stack paddingInline={4}>
            <Text color="secondary">{t("loading")}</Text>
          </Stack>
        ) : snapshots.length === 0 ? (
          <EmptyState isCompact title={t("empty")} />
        ) : (
          <Table<SaveSnapshot & Record<string, unknown>>
            data={snapshots as (SaveSnapshot & Record<string, unknown>)[]}
            idKey="id"
            density="compact"
            columns={[
              {
                key: "createdAt",
                header: t("columns.taken"),
                width: proportional(2),
                renderCell: (x) => <Timestamp value={x.createdAt} format="date_time" />,
              },
              {
                key: "source",
                header: t("columns.source"),
                width: proportional(2),
                renderCell: (x) => t(`sources.${x.source}`),
              },
              {
                key: "rows",
                header: t("columns.rows"),
                width: pixel(80),
                renderCell: (x) => String(x.rows),
              },
              {
                key: "expiresAt",
                header: t("columns.expires"),
                width: proportional(2),
                renderCell: (x) => <Timestamp value={x.expiresAt} format="relative_short" />,
              },
              {
                key: "id",
                header: "",
                width: pixel(120),
                renderCell: (x) =>
                  x.restoredAt ? (
                    <Badge variant="neutral" label={t("restoredBadge")} />
                  ) : (
                    <Button label={t("restore")} size="sm" onClick={() => setPicked(x)} />
                  ),
              },
            ]}
          />
        )}
        <Stack direction="horizontal" gap={2} justify="end" paddingInline={4} paddingBlockEnd={4}>
          {picked ? (
            <>
              <Button label={t("back")} variant="ghost" onClick={() => setPicked(null)} />
              <Button
                label={t("confirm")}
                variant="destructive"
                isDisabled={reason.trim().length < 3}
                isLoading={busy}
                onClick={restore}
              />
            </>
          ) : (
            <Button label={t("close")} variant="ghost" onClick={onClose} />
          )}
        </Stack>
      </Stack>
    </Dialog>
  );
}
