"use client";

import { AlertDialog } from "@astryxdesign/core/AlertDialog";
import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Link } from "@astryxdesign/core/Link";
import { MetadataList, MetadataListItem } from "@astryxdesign/core/MetadataList";
import { Stack } from "@astryxdesign/core/Stack";
import { pixel, proportional, Table, useTableRowExpansion } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Undo2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { revertAction } from "@/app/(dashboard)/audit/actions";
import type { AuditEvent } from "@/lib/db/schema";
import { useAuditText } from "./AuditSentence";
import { ControllerLabel, useControllers } from "./ControllerContext";
import { useAction } from "./useAction";

type Row = AuditEvent & Record<string, unknown>;

// Actions revert.ts holds an inverse for; kept here so the button only shows where it can work.
const REVERTIBLE = new Set([
  "player.ban",
  "player.timeout",
  "player.unban",
  "player.lift_timeout",
  "player.access",
  "player.roles",
  "filter.term.create",
  "filter.term.update",
  "filter.term.delete",
  "chat_filter.restore",
  "settings.restore",
  "staff.restore",
  "settings.update",
  "staff.create",
  "staff.update",
  "report.update",
  "controller.update",
  "controller.logs",
]);

function isFlat(v: unknown): v is Record<string, unknown> {
  return Boolean(v) && typeof v === "object" && !Array.isArray(v);
}

/**
 * Where a filter hit's line is in the chat log. Direct messages have none: they are read only with a reason,
 * from the player's Direct tab.
 */
function chatLine(e: AuditEvent): string | null {
  if (!e.action.startsWith("chat.filter_") || !isFlat(e.after) || !e.targetId) return null;
  const { channel, at } = e.after;
  if (channel === "direct" || typeof at !== "number") return null;
  const params = new URLSearchParams({
    user: e.targetId,
    flagged: "1",
    before: String(at + 1),
    c: e.controllerId,
  });
  return `/chat?${params}`;
}

const show = (v: unknown) =>
  v === null || v === undefined ? "—" : typeof v === "object" ? JSON.stringify(v) : String(v);

/** Field by field, what changed: the old value struck through beside the new one. */
export function AuditDiff({ before, after }: { before: unknown; after: unknown }) {
  const t = useTranslations("audit");
  if (!isFlat(before) && !isFlat(after)) return <Text type="supporting">{t("noDetail")}</Text>;
  const b = isFlat(before) ? before : {};
  const a = isFlat(after) ? after : {};
  const keys = [...new Set([...Object.keys(b), ...Object.keys(a)])].filter(
    (k) => k !== "revisionId" && k !== "updatedAt" && k !== "createdAt",
  );
  const changed = keys.filter((k) => JSON.stringify(b[k]) !== JSON.stringify(a[k]));
  const rows = changed.length ? changed : keys;
  if (rows.length === 0) return <Text type="supporting">{t("noDetail")}</Text>;
  return (
    <MetadataList label={{ position: "start", width: 160 }}>
      {rows.map((k) => (
        <MetadataListItem key={k} label={k}>
          <Stack direction="horizontal" gap={2} wrap="wrap">
            {k in b && (
              <span className={k in a ? "dainty-diff-before dash-json" : "dash-json"}>
                {show(b[k])}
              </span>
            )}
            {k in a && <span className="dainty-diff-after dash-json">{show(a[k])}</span>}
          </Stack>
        </MetadataListItem>
      ))}
    </MetadataList>
  );
}

export function AuditTable({
  events,
  canRevert,
  emptyTitle,
}: {
  events: AuditEvent[];
  canRevert: boolean;
  emptyTitle?: string;
}) {
  const t = useTranslations("audit");
  const tf = useTranslations("fleet");
  const text = useAuditText();
  const { run } = useAction();
  const { fleetWide } = useControllers();
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [confirm, setConfirm] = useState<AuditEvent | null>(null);
  const [busy, setBusy] = useState(false);

  const expansion = useTableRowExpansion<Row>({
    expandedKeys: expanded,
    onToggle: (key) =>
      setExpanded((prev) => {
        const next = new Set(prev);
        if (next.has(key)) next.delete(key);
        else next.add(key);
        return next;
      }),
    getRowKey: (e) => String(e.id),
    getIsItemExpandable: (e) => e.before !== null || e.after !== null || Boolean(e.reason),
    renderExpanded: (e) => (
      <Stack gap={3} paddingBlock={2}>
        {e.reason && (
          <Text>
            <Text weight="medium">{t("reason")}</Text> {e.reason}
          </Text>
        )}
        <AuditDiff before={e.before} after={e.after} />
        {chatLine(e) && <Link href={chatLine(e) ?? ""}>{t("openChatLine")}</Link>}
        <Text type="supporting">
          {t("eventId", { id: e.id })}
          {e.ip ? ` · ${e.ip}` : ""}
        </Text>
      </Stack>
    ),
  });

  if (events.length === 0) return <EmptyState isCompact title={emptyTitle ?? t("empty")} />;

  return (
    <>
      <Table<Row>
        data={events as Row[]}
        idKey={(e) => String(e.id)}
        density="compact"
        hasHover
        plugins={{ expansion }}
        columns={[
          {
            key: "at",
            header: t("columns.when"),
            width: pixel(150),
            renderCell: (e) => <Timestamp value={new Date(e.at).toISOString()} format="auto" />,
          },
          {
            key: "action",
            header: t("columns.change"),
            width: proportional(4),
            renderCell: (e) => (
              <Stack gap={0.5}>
                <Text>{text(e)}</Text>
                <Stack direction="horizontal" gap={1}>
                  {e.id < 0 && <Badge variant="neutral" label={t("fromGame")} />}
                  {e.revertOfId && (
                    <Badge variant="info" label={t("undoes", { id: e.revertOfId })} />
                  )}
                  {e.revertedById && (
                    <Badge variant="neutral" label={t("undone", { id: e.revertedById })} />
                  )}
                </Stack>
              </Stack>
            ),
          },
          {
            key: "targetLabel",
            header: t("columns.target"),
            width: proportional(1),
            renderCell: (e) =>
              e.targetType === "player" && e.targetLabel ? (
                <Link href={`/players/${encodeURIComponent(e.targetLabel)}?c=${e.controllerId}`}>
                  {e.targetLabel}
                </Link>
              ) : e.targetType === "report" && e.targetId ? (
                <Link href={`/reports/${e.targetId}`}>{t("reportRef", { id: e.targetId })}</Link>
              ) : (
                <Text type="supporting">{t(`targets.${e.targetType as "player"}`)}</Text>
              ),
          },
          ...(fleetWide
            ? [
                {
                  key: "controllerId",
                  header: tf("columns.controller"),
                  width: pixel(140),
                  renderCell: (e: Row) => <ControllerLabel id={e.controllerId} />,
                },
              ]
            : []),
          {
            key: "revert",
            header: <Text type="label">{t("columns.undo")}</Text>,
            width: pixel(110),
            align: "end",
            renderCell: (e) =>
              canRevert && e.revertible && !e.revertedById && REVERTIBLE.has(e.action) ? (
                <Button
                  label={t("undo")}
                  size="sm"
                  variant="ghost"
                  icon={<Undo2 size={14} />}
                  onClick={() => setConfirm(e)}
                />
              ) : null,
          },
        ]}
      />
      <AlertDialog
        isOpen={confirm !== null}
        onOpenChange={(open) => !open && setConfirm(null)}
        title={t("confirmTitle")}
        description={confirm ? t("confirmBody", { change: text(confirm) }) : ""}
        actionLabel={t("undo")}
        actionVariant="primary"
        isActionLoading={busy}
        onAction={async () => {
          if (!confirm) return;
          setBusy(true);
          const ok = await run(() => revertAction(confirm.id), t("undoneToast"));
          setBusy(false);
          if (ok !== false) setConfirm(null);
        }}
      />
    </>
  );
}
