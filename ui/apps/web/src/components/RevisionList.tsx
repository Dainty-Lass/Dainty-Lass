"use client";

import { AlertDialog } from "@astryxdesign/core/AlertDialog";
import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Stack } from "@astryxdesign/core/Stack";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { History } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { restoreRevisionAction } from "@/app/(dashboard)/audit/actions";
import type { ConfigRevision } from "@/lib/db/schema";
import { useAction } from "./useAction";

export type RevisionRow = Omit<ConfigRevision, "snapshot"> & { size?: number };

/** Version history for one kind of config; restoring writes the old version as the newest one. */
export function RevisionList({
  revisions,
  canRestore,
}: {
  revisions: RevisionRow[];
  canRestore: boolean;
}) {
  const t = useTranslations("revisions");
  const { run } = useAction();
  const [target, setTarget] = useState<RevisionRow | null>(null);
  const [busy, setBusy] = useState(false);
  const latest = revisions[0]?.version;

  // "add:spam" → "Added spam": the verb is a catalog key, the subject is data.
  const describe = (summary: string) => {
    const [verb, ...rest] = summary.split(":");
    const subject = rest.join(":");
    return t.has(`verbs.${verb}` as never)
      ? t(`verbs.${verb}` as never, { subject } as never)
      : summary;
  };

  if (revisions.length === 0) return <EmptyState isCompact title={t("empty")} />;

  return (
    <>
      <Table<RevisionRow & Record<string, unknown>>
        data={revisions as (RevisionRow & Record<string, unknown>)[]}
        idKey="id"
        density="compact"
        columns={[
          {
            key: "version",
            header: t("version"),
            width: pixel(100),
            renderCell: (r) => (
              <Stack direction="horizontal" gap={1} align="center">
                <Text type="code">v{r.version}</Text>
                {r.version === latest && <Badge variant="success" label={t("current")} />}
              </Stack>
            ),
          },
          {
            key: "summary",
            header: t("change"),
            width: proportional(3),
            renderCell: (r) => describe(r.summary),
          },
          { key: "actorName", header: t("by"), width: pixel(140) },
          {
            key: "at",
            header: t("when"),
            width: pixel(130),
            renderCell: (r) => (
              <Timestamp value={new Date(r.at).toISOString()} format="relative_short" />
            ),
          },
          {
            key: "restore",
            header: "",
            width: pixel(110),
            align: "end",
            renderCell: (r) =>
              canRestore && r.version !== latest ? (
                <Button
                  label={t("restore")}
                  size="sm"
                  variant="ghost"
                  icon={<History size={14} />}
                  onClick={() => setTarget(r)}
                />
              ) : null,
          },
        ]}
      />
      <AlertDialog
        isOpen={target !== null}
        onOpenChange={(open) => !open && setTarget(null)}
        title={t("confirmTitle", { version: target?.version ?? 0 })}
        description={t("confirmBody")}
        actionLabel={t("restore")}
        actionVariant="primary"
        isActionLoading={busy}
        onAction={async () => {
          if (!target) return;
          setBusy(true);
          const ok = await run(
            () => restoreRevisionAction(target.id),
            t("restored", { version: target.version }),
          );
          setBusy(false);
          if (ok !== false) setTarget(null);
        }}
      />
    </>
  );
}
