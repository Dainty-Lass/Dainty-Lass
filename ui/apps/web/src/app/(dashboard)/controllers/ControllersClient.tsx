"use client";

import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Link } from "@astryxdesign/core/Link";
import { Pagination } from "@astryxdesign/core/Pagination";
import { Selector } from "@astryxdesign/core/Selector";
import { Stack } from "@astryxdesign/core/Stack";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Token } from "@astryxdesign/core/Token";
import { Toolbar } from "@astryxdesign/core/Toolbar";
import { StatTiles } from "@dainty/theme";
import { Plus } from "lucide-react";
import { useRouter } from "next/navigation";
import { useFormatter, useTranslations } from "next-intl";
import { useMemo, useState } from "react";
import { ControllerState, STATE_ORDER } from "@/components/ControllerState";
import { InstallsCard } from "@/components/GameServers";
import { Page } from "@/components/Page";
import { SearchField } from "@/components/SearchField";

import { useAction } from "@/components/useAction";
import { useQueryPatch } from "@/components/useQueryPatch";
import type { GamesReport } from "@/lib/fleet/games";
import type { ControllerView, FleetTotals } from "@/lib/fleet/status";
import { createControllerAction } from "./actions";
import { PairingInstructions } from "./PairingInstructions";

type Row = ControllerView & Record<string, unknown>;
const PER_PAGE = 50;
const STATE_FILTERS = ["down", "online", "pending", "disabled"] as const;

function matchesState(v: ControllerView, filter: string): boolean {
  if (!filter) return true;
  if (filter === "down") return v.state === "offline" || v.state === "stale";
  return v.state === filter;
}

export function ControllersClient({
  controllers,
  games,
  totals,
  filters,
  canWrite,
  dashboardUrl,
  insecure,
}: {
  controllers: ControllerView[];
  /** The remote controller's installs (D1), beside this dashboard. */
  games: GamesReport | null;
  totals: FleetTotals;
  filters: { state: string; q: string; region: string };
  canWrite: boolean;
  dashboardUrl: string;
  insecure: boolean;
}) {
  const t = useTranslations("fleet");
  const tc = useTranslations("common");
  const format = useFormatter();
  const patch = useQueryPatch();
  const router = useRouter();
  const { run } = useAction();
  const [page, setPage] = useState(1);
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [region, setRegion] = useState("");
  const [paired, setPaired] = useState<{
    id: string;
    code: string;
    expiresAt: number | null;
  } | null>(null);

  const regions = useMemo(
    () =>
      [...new Set(controllers.map((c) => c.region).filter((r): r is string => Boolean(r)))].sort(),
    [controllers],
  );
  const shown = controllers
    .filter((c) => matchesState(c, filters.state))
    .filter((c) => !filters.region || c.region === filters.region)
    .filter((c) => !filters.q || c.name.toLowerCase().includes(filters.q.toLowerCase()))
    .sort((a, b) => STATE_ORDER[a.state] - STATE_ORDER[b.state] || a.name.localeCompare(b.name));
  const pageRows = shown.slice((page - 1) * PER_PAGE, page * PER_PAGE);
  const down = controllers.filter((c) => c.state === "offline" || c.state === "stale").length;

  return (
    <Page
      title={t("title")}
      description={t("description")}
      actions={
        canWrite ? (
          <Button
            label={t("add")}
            variant="primary"
            icon={<Plus size={16} />}
            onClick={() => setAdding(true)}
          />
        ) : undefined
      }
    >
      <StatTiles
        tiles={[
          {
            id: "online",
            label: t("controllersOnline"),
            value: `${totals.online}/${totals.controllers}`,
            hue: down ? "red" : "cyan",
            note: down ? t("downCount", { count: down }) : t("allUp"),
          },
          {
            id: "players",
            label: t("columns.players"),
            value: format.number(totals.players),
            hue: "green",
          },
          {
            id: "hunts",
            label: t("columns.hunts"),
            value: `${format.number(totals.huntsRunning)}/${format.number(totals.huntsMax)}`,
            hue: "purple",
          },
        ]}
      />

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
              label={t("columns.state")}
              isLabelHidden
              size="sm"
              variant="secondary"
              hasClear
              placeholder={t("anyState")}
              value={filters.state || null}
              options={STATE_FILTERS.map((s) => ({ value: s, label: t(`stateFilters.${s}`) }))}
              onChange={(v) => {
                setPage(1);
                patch({ state: v });
              }}
            />
            {regions.length > 0 && (
              <Selector
                label={t("region")}
                isLabelHidden
                size="sm"
                variant="secondary"
                hasClear
                placeholder={t("anyRegion")}
                value={filters.region || null}
                options={regions.map((r) => ({ value: r, label: r }))}
                onChange={(v) => {
                  setPage(1);
                  patch({ region: v });
                }}
              />
            )}
          </>
        }
        endContent={
          <Text type="supporting">
            {t("shown", { shown: shown.length, total: controllers.length })}
          </Text>
        }
      />

      <Card padding={0}>
        {pageRows.length === 0 ? (
          <EmptyState title={t("noMatches")} />
        ) : (
          <Table<Row>
            data={pageRows as Row[]}
            idKey="id"
            hasHover
            density="compact"
            columns={[
              {
                key: "name",
                header: t("columns.controller"),
                width: proportional(2),
                renderCell: (c) => (
                  <Stack gap={0}>
                    <Link href={`/controllers/${c.id}`}>{c.name}</Link>
                    <Text type="supporting" size="xsm">
                      {c.kind === "local"
                        ? t("thisHost")
                        : (c.status?.server?.name ?? c.id.slice(0, 8))}
                    </Text>
                  </Stack>
                ),
              },
              {
                key: "region",
                header: t("region"),
                width: pixel(120),
                renderCell: (c) => (c.region ? <Token size="sm" label={c.region} /> : "—"),
              },
              {
                key: "state",
                header: t("columns.state"),
                width: pixel(140),
                renderCell: (c) => <ControllerState state={c.state} />,
              },
              {
                key: "players",
                header: t("columns.players"),
                width: pixel(90),
                align: "end",
                renderCell: (c) => format.number(c.status?.online ?? 0),
              },
              {
                key: "hunts",
                header: t("columns.hunts"),
                width: pixel(90),
                align: "end",
                renderCell: (c) =>
                  `${c.status?.hunts?.running ?? 0}/${c.status?.hunts?.maxHuntServers ?? 0}`,
              },
              {
                key: "version",
                header: t("columns.version"),
                width: pixel(130),
                renderCell: (c) => (
                  <Text type="code">{c.status?.server?.version ?? c.version ?? "—"}</Text>
                ),
              },
              {
                key: "lastSeenAt",
                header: t("columns.lastSeen"),
                width: pixel(130),
                renderCell: (c) =>
                  c.lastSeenAt ? (
                    <Timestamp
                      value={new Date(c.lastSeenAt).toISOString()}
                      format="relative_short"
                      isLive
                    />
                  ) : (
                    t("never")
                  ),
              },
            ]}
          />
        )}
      </Card>
      {shown.length > PER_PAGE && (
        <Stack align="center">
          <Pagination
            page={page}
            totalItems={shown.length}
            pageSize={PER_PAGE}
            onChange={setPage}
          />
        </Stack>
      )}

      <InstallsCard report={games} />

      <Dialog
        isOpen={adding}
        onOpenChange={(open) => {
          if (open) return;
          setAdding(false);
          if (paired) router.push(`/controllers/${paired.id}`);
          setPaired(null);
        }}
        purpose="form"
        width={560}
      >
        <Stack gap={4}>
          <DialogHeader
            title={paired ? t("pairing.title", { name }) : t("addTitle")}
            subtitle={paired ? undefined : t("addExplain")}
            onOpenChange={() => setAdding(false)}
          />
          <Stack gap={4} paddingInline={4} paddingBlockEnd={4}>
            {paired ? (
              <PairingInstructions
                code={paired.code}
                expiresAt={paired.expiresAt}
                dashboardUrl={dashboardUrl}
                insecure={insecure}
              />
            ) : (
              <>
                <TextInput
                  label={t("name")}
                  value={name}
                  onChange={setName}
                  isRequired
                  hasAutoFocus
                  placeholder={t("namePlaceholder")}
                />
                <TextInput
                  label={t("region")}
                  value={region}
                  onChange={setRegion}
                  isOptional
                  placeholder={t("regionPlaceholder")}
                />
                <Stack direction="horizontal" gap={2} justify="end">
                  <Button label={tc("cancel")} variant="ghost" onClick={() => setAdding(false)} />
                  <Button
                    label={t("createAndPair")}
                    variant="primary"
                    isDisabled={!name.trim()}
                    clickAction={async () => {
                      const r = await run(() => createControllerAction(name, region));
                      if (r !== false)
                        setPaired(r as { id: string; code: string; expiresAt: number | null });
                    }}
                  />
                </Stack>
              </>
            )}
          </Stack>
        </Stack>
      </Dialog>
    </Page>
  );
}
