"use client";

import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Pagination } from "@astryxdesign/core/Pagination";
import { Selector } from "@astryxdesign/core/Selector";
import { Switch } from "@astryxdesign/core/Switch";
import { Text } from "@astryxdesign/core/Text";
import { Toolbar } from "@astryxdesign/core/Toolbar";
import { Download } from "lucide-react";
import { useTranslations } from "next-intl";
import { AuditTable } from "@/components/AuditTable";
import { Page } from "@/components/Page";
import { SearchField } from "@/components/SearchField";
import { useQueryPatch } from "@/components/useQueryPatch";
import type { AuditEvent } from "@/lib/db/schema";

const TARGETS = [
  "player",
  "report",
  "filter_term",
  "chat_filter",
  "staff",
  "settings",
  "session",
] as const;
const ACTION_GROUPS = [
  "player.",
  "report.",
  "filter.",
  "chat.direct_read",
  "staff.",
  "settings.",
  "session.",
] as const;
const RANGES = ["1h", "24h", "7d", "30d"] as const;

export function AuditClient({
  events,
  total,
  page,
  actors,
  filters,
  canRevert,
  exportHref,
}: {
  events: AuditEvent[];
  total: number;
  page: number;
  actors: { id: number; name: string }[];
  filters: {
    actor: string;
    target: string;
    action: string;
    q: string;
    range: string;
    revertible: boolean;
  };
  canRevert: boolean;
  exportHref: string;
}) {
  const t = useTranslations("audit");
  const tc = useTranslations("common");
  const patch = useQueryPatch();

  return (
    <Page
      title={t("title")}
      description={t("description")}
      actions={<Button label={tc("export")} icon={<Download size={16} />} href={exportHref} />}
    >
      <Toolbar
        label={t("filters")}
        startContent={
          <>
            <SearchField
              label={tc("search")}
              placeholder={t("searchPlaceholder")}
              value={filters.q}
              onCommit={(q) => patch({ q })}
            />
            <Selector
              label={t("columns.who")}
              isLabelHidden
              size="sm"
              variant="secondary"
              hasClear
              placeholder={t("anyone")}
              value={filters.actor || null}
              options={actors.map((a) => ({ value: String(a.id), label: a.name }))}
              onChange={(v) => patch({ actor: v })}
            />
            <Selector
              label={t("columns.target")}
              isLabelHidden
              size="sm"
              variant="secondary"
              hasClear
              placeholder={t("anyTarget")}
              value={filters.target || null}
              options={TARGETS.map((x) => ({ value: x, label: t(`targets.${x}`) }))}
              onChange={(v) => patch({ target: v })}
            />
            <Selector
              label={t("columns.change")}
              isLabelHidden
              size="sm"
              variant="secondary"
              hasClear
              placeholder={t("anyAction")}
              value={filters.action || null}
              options={ACTION_GROUPS.map((a) => ({
                value: a,
                label: t(`groups.${a.replace(/\.$/, "").replace(".", "_") as "player"}`),
              }))}
              onChange={(v) => patch({ action: v })}
            />
            <Selector
              label={tc("ranges.label")}
              isLabelHidden
              size="sm"
              variant="secondary"
              value={filters.range || "all"}
              options={[
                ...RANGES.map((r) => ({ value: r, label: tc(`ranges.${r}`) })),
                { value: "all", label: tc("ranges.all") },
              ]}
              onChange={(v) => patch({ range: v === "all" ? null : v })}
            />
          </>
        }
        endContent={
          <Switch
            label={t("undoableOnly")}
            value={filters.revertible}
            onChange={(on) => patch({ revertible: on })}
            size="sm"
          />
        }
      />
      <Text type="supporting">{t("count", { count: total })}</Text>
      <Card padding={0}>
        <AuditTable events={events} canRevert={canRevert} />
      </Card>
      {total > 50 && (
        <Pagination
          page={page}
          totalItems={total}
          pageSize={50}
          onChange={(p) => patch({ page: p })}
        />
      )}
    </Page>
  );
}
