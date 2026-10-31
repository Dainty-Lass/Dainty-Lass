"use client";

import { Badge } from "@astryxdesign/core/Badge";
import { Banner } from "@astryxdesign/core/Banner";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Link } from "@astryxdesign/core/Link";
import { Pagination } from "@astryxdesign/core/Pagination";
import { SegmentedControl, SegmentedControlItem } from "@astryxdesign/core/SegmentedControl";
import { Selector } from "@astryxdesign/core/Selector";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Token } from "@astryxdesign/core/Token";
import { Toolbar } from "@astryxdesign/core/Toolbar";
import { useTranslations } from "next-intl";
import { Page } from "@/components/Page";
import { SearchField } from "@/components/SearchField";
import { useQueryPatch } from "@/components/useQueryPatch";
import type { KnownPlayer } from "@/lib/directory";
import type { BackendAccount } from "@/lib/game/backend";

const STATUS_BADGE = { active: "success", disabled: "neutral", banned: "error" } as const;

type Row = BackendAccount & Record<string, unknown>;

export function PlayersClient({
  accounts,
  total,
  page,
  onlineIds,
  failure,
  known,
  filters,
}: {
  accounts: BackendAccount[] | null;
  total: number;
  page: number;
  onlineIds: string[];
  failure: string | null;
  known: KnownPlayer[];
  filters: { q: string; status: string; online: boolean };
}) {
  const t = useTranslations("players");
  const te = useTranslations("errors");
  const patch = useQueryPatch();
  const online = new Set(onlineIds);

  return (
    <Page title={t("title")} description={t("description")}>
      <Toolbar
        label={t("filters")}
        startContent={
          <>
            <SegmentedControl
              label={t("view")}
              value={filters.online ? "online" : "all"}
              onChange={(v) => patch({ online: v === "online" })}
              size="sm"
            >
              <SegmentedControlItem value="all" label={t("allPlayers")} />
              <SegmentedControlItem
                value="online"
                label={t("onlineNow", { count: onlineIds.length })}
              />
            </SegmentedControl>
            <SearchField
              label={t("search")}
              placeholder={t("searchPlaceholder")}
              value={filters.q}
              onCommit={(q) => patch({ q })}
            />
            <Selector
              label={t("status")}
              isLabelHidden
              size="sm"
              variant="secondary"
              placeholder={t("anyStatus")}
              hasClear
              value={filters.status || null}
              options={(["active", "disabled", "banned"] as const).map((s) => ({
                value: s,
                label: t(`statuses.${s}`),
              }))}
              onChange={(v) => patch({ status: v })}
            />
          </>
        }
      />

      {failure && (
        <Banner
          status="warning"
          title={t("backendUnavailable")}
          description={te(failure as "unknown", { detail: "DASHBOARD_MANAGER_OPERATOR_SECRET" })}
          collapsible={false}
        />
      )}

      {accounts ? (
        <Card padding={0}>
          {accounts.length === 0 ? (
            <EmptyState title={t("noMatches")} />
          ) : (
            <Table<Row>
              data={accounts as Row[]}
              idKey="userId"
              hasHover
              density="balanced"
              columns={[
                {
                  key: "displayName",
                  header: t("columns.player"),
                  width: proportional(2),
                  renderCell: (a) => (
                    <Stack direction="horizontal" gap={2} align="center">
                      <StatusDot
                        variant={online.has(a.userId) ? "success" : "neutral"}
                        label={online.has(a.userId) ? t("online") : t("offline")}
                      />
                      <Stack gap={0}>
                        <Link href={`/players/${encodeURIComponent(a.displayName)}`}>
                          {a.displayName}
                        </Link>
                        <Text type="supporting" size="xsm">
                          {a.userId}
                        </Text>
                      </Stack>
                    </Stack>
                  ),
                },
                {
                  key: "status",
                  header: t("columns.status"),
                  width: pixel(120),
                  renderCell: (a) => (
                    <Badge variant={STATUS_BADGE[a.status]} label={t(`statuses.${a.status}`)} />
                  ),
                },
                {
                  key: "roles",
                  header: t("columns.roles"),
                  width: proportional(1),
                  renderCell: (a) => (
                    <Stack direction="horizontal" gap={1} wrap="wrap">
                      {a.roles.length ? (
                        a.roles.map((r) => <Token key={r} size="sm" label={r} />)
                      ) : (
                        <Text type="supporting">—</Text>
                      )}
                    </Stack>
                  ),
                },
                {
                  key: "approvalStatus",
                  header: t("columns.approval"),
                  width: pixel(120),
                  renderCell: (a) => t(`approval.${a.approvalStatus}`),
                },
                {
                  key: "lastLoginAt",
                  header: t("columns.lastLogin"),
                  width: pixel(140),
                  renderCell: (a) =>
                    a.lastLoginAt ? (
                      <Timestamp value={a.lastLoginAt} format="relative_short" />
                    ) : (
                      "—"
                    ),
                },
                {
                  key: "createdAt",
                  header: t("columns.joined"),
                  width: pixel(140),
                  renderCell: (a) => <Timestamp value={a.createdAt} format="date" />,
                },
              ]}
            />
          )}
        </Card>
      ) : (
        <Card padding={0}>
          <Stack paddingInline={4} paddingBlock={3}>
            <Text type="supporting">{t("fromChatLog")}</Text>
          </Stack>
          {known.length === 0 ? (
            <EmptyState title={t("noneKnown")} />
          ) : (
            <Table<KnownPlayer & Record<string, unknown>>
              data={known as (KnownPlayer & Record<string, unknown>)[]}
              idKey="userId"
              hasHover
              columns={[
                {
                  key: "name",
                  header: t("columns.player"),
                  renderCell: (p) => (
                    <Link href={`/players/${encodeURIComponent(p.name)}`}>{p.name}</Link>
                  ),
                },
                { key: "messages", header: t("columns.messages"), width: pixel(120), align: "end" },
                {
                  key: "lastSeenAt",
                  header: t("columns.lastSeen"),
                  width: pixel(160),
                  renderCell: (p) => (
                    <Timestamp
                      value={new Date(p.lastSeenAt).toISOString()}
                      format="relative_short"
                    />
                  ),
                },
              ]}
            />
          )}
        </Card>
      )}

      {accounts && !filters.online && total > 25 && (
        <Pagination
          page={page}
          totalItems={total}
          pageSize={25}
          onChange={(p) => patch({ page: p })}
        />
      )}
    </Page>
  );
}
