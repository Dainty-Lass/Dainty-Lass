"use client";

import { AlertDialog } from "@astryxdesign/core/AlertDialog";
import { Banner } from "@astryxdesign/core/Banner";
import { BreadcrumbItem, Breadcrumbs } from "@astryxdesign/core/Breadcrumbs";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { DropdownMenu } from "@astryxdesign/core/DropdownMenu";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Grid } from "@astryxdesign/core/Grid";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { MetadataList, MetadataListItem } from "@astryxdesign/core/MetadataList";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { TextArea } from "@astryxdesign/core/TextArea";
import { TextInput } from "@astryxdesign/core/TextInput";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Token } from "@astryxdesign/core/Token";
import type { Hunt } from "@dainty/fleet";
import { StatTiles } from "@dainty/theme";
import { Crosshair } from "lucide-react";
import { useRouter } from "next/navigation";
import { useFormatter, useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { AuditTable } from "@/components/AuditTable";
import { ControllerState } from "@/components/ControllerState";
import { Page } from "@/components/Page";
import { PerformanceSection } from "@/components/PerformanceSection";
import { Sparkline } from "@/components/Sparkline";

import { useAction } from "@/components/useAction";
import type { AuditEvent } from "@/lib/db/schema";
import type { Performance } from "@/lib/fleet/metrics";
import type { ControllerView } from "@/lib/fleet/status";
import {
  removeControllerAction,
  repairControllerAction,
  setScopeAction,
  updateControllerAction,
} from "../actions";
import { PairingInstructions } from "../PairingInstructions";

const SERVICE_DOT = { ok: "success", down: "error", not_configured: "neutral" } as const;
type HuntRow = Hunt & Record<string, unknown>;

export function ControllerDetailClient({
  controller: c,
  perf,
  history,
  reports,
  audit,
  inScope,
  canWrite,
  canRevert,
  dashboardUrl,
  insecure,
}: {
  controller: ControllerView;
  perf: Performance;
  history: { at: number; online: number; hunts: number }[];
  // Null where the reader's role may not see them.
  reports: Record<"open" | "claimed" | "resolved" | "dismissed", number> | null;
  audit: AuditEvent[] | null;
  inScope: boolean;
  canWrite: boolean;
  canRevert: boolean;
  dashboardUrl: string;
  insecure: boolean;
}) {
  const t = useTranslations("fleet");
  const to = useTranslations("overview");
  const tc = useTranslations("common");
  const format = useFormatter();
  const router = useRouter();
  const { run } = useAction();
  const [editing, setEditing] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [code, setCode] = useState<string | null>(null);
  const [name, setName] = useState(c.name);
  const [region, setRegion] = useState(c.region ?? "");
  const [note, setNote] = useState(c.note ?? "");

  useEffect(() => {
    const id = setInterval(() => router.refresh(), 15_000);
    return () => clearInterval(id);
  }, [router]);

  const s = c.status;
  const hunts = (s?.hunts?.hunts ?? []) as HuntRow[];
  const agent = c.kind === "agent";

  return (
    <Page
      title={
        <Stack direction="horizontal" gap={2} align="center">
          {c.name}
          <ControllerState state={c.state} />
        </Stack>
      }
      description={
        agent ? t("agentDescription", { region: c.region ?? "—" }) : t("localDescription")
      }
      breadcrumbs={
        <Breadcrumbs variant="supporting">
          <BreadcrumbItem href="/controllers">{t("title")}</BreadcrumbItem>
          <BreadcrumbItem>{c.name}</BreadcrumbItem>
        </Breadcrumbs>
      }
      actions={
        <>
          <Button
            label={inScope ? t("inScope") : t("focus")}
            icon={<Crosshair size={16} />}
            isDisabled={inScope}
            clickAction={async () =>
              void (await run(() => setScopeAction(c.id), t("focused", { name: c.name })))
            }
          />
          {canWrite && (
            <DropdownMenu
              button={{ label: tc("actions"), variant: "secondary" }}
              items={[
                { label: t("edit"), onClick: () => setEditing(true) },
                ...(agent
                  ? [
                      {
                        label: t("repair"),
                        onClick: async () => {
                          const r = await run(() => repairControllerAction(c.id));
                          if (r !== false) setCode(r as string);
                        },
                      },
                      {
                        label: c.state === "disabled" ? t("enable") : t("disable"),
                        onClick: () =>
                          void run(() =>
                            updateControllerAction(c.id, { disabled: c.state !== "disabled" }),
                          ),
                      },
                      {
                        label: t("remove"),
                        variant: "destructive" as const,
                        onClick: () => setRemoving(true),
                      },
                    ]
                  : []),
              ]}
            />
          )}
        </>
      }
    >
      {c.state === "pending" && (
        <Banner
          status="info"
          title={t("pendingBanner")}
          description={t("pendingHint")}
          collapsible={false}
        />
      )}
      {(c.state === "offline" || c.state === "stale") && (
        <Banner
          status="error"
          title={t(c.state === "offline" ? "offlineBanner" : "staleBanner")}
          description={c.lastSeenAt ? undefined : t("neverSeen")}
          collapsible={false}
        />
      )}
      {c.state === "not_configured" && (
        <Banner status="warning" title={t("localNotConfigured")} collapsible={false} />
      )}

      <StatTiles
        tiles={[
          {
            id: "players",
            label: to("stats.online"),
            value: format.number(s?.online ?? 0),
            hue: "green",
            note: s?.server
              ? to("stats.uptime", {
                  hours: format.number(Math.floor(s.server.uptimeSeconds / 3600)),
                })
              : undefined,
          },
          {
            id: "hunts",
            label: to("stats.hunts"),
            value: `${s?.hunts?.running ?? 0}/${s?.hunts?.maxHuntServers ?? 0}`,
            hue: "purple",
            note: s?.hunts ? to("stats.huntsFree", { free: s.hunts.free }) : undefined,
          },
          ...(reports
            ? [
                {
                  id: "reports",
                  label: to("stats.openReports"),
                  href: "/reports",
                  value: format.number(reports.open),
                  hue: reports.open ? ("orange" as const) : ("gray" as const),
                  note: to("stats.claimed", { count: reports.claimed }),
                },
              ]
            : []),
        ]}
      />

      <Grid columns={{ minWidth: 340, repeat: "fit" }} gap={4}>
        <Card padding={4}>
          <Stack gap={3}>
            <Heading level={2}>{to("health")}</Heading>
            <List>
              <ListItem
                label={to("services.adminApi")}
                description={
                  s?.server
                    ? to("services.version", {
                        version: s.server.version,
                        commit: s.server.commit ?? "—",
                      })
                    : undefined
                }
                startContent={
                  <StatusDot
                    variant={SERVICE_DOT[s?.adminApi ?? "down"]}
                    label={to(`state.${s?.adminApi ?? "down"}`)}
                  />
                }
                endContent={<Text type="supporting">{to(`state.${s?.adminApi ?? "down"}`)}</Text>}
              />
              <ListItem
                label={to("services.backend")}
                startContent={
                  <StatusDot
                    variant={SERVICE_DOT[s?.backend ?? "down"]}
                    label={to(`state.${s?.backend ?? "down"}`)}
                  />
                }
                endContent={<Text type="supporting">{to(`state.${s?.backend ?? "down"}`)}</Text>}
              />
            </List>
            <MetadataList label={{ position: "start", width: 150 }}>
              {s?.server && (
                <MetadataListItem label={t("serverName")}>{s.server.name}</MetadataListItem>
              )}
              {s?.server && (
                <MetadataListItem label={t("mode")}>
                  {tc(`modes.${s.server.mode}`)}
                </MetadataListItem>
              )}
              <MetadataListItem label={t("columns.version")}>
                <Text type="code">{c.version ?? "—"}</Text>
              </MetadataListItem>
              <MetadataListItem label={t("columns.lastSeen")}>
                {c.lastSeenAt ? (
                  <Timestamp
                    value={new Date(c.lastSeenAt).toISOString()}
                    format="relative"
                    isLive
                  />
                ) : (
                  t("never")
                )}
              </MetadataListItem>
              {agent && (
                <MetadataListItem label={t("connectedSince")}>
                  {c.connectedSince ? (
                    <Timestamp value={new Date(c.connectedSince).toISOString()} format="relative" />
                  ) : (
                    "—"
                  )}
                </MetadataListItem>
              )}
              {c.region && (
                <MetadataListItem label={t("region")}>
                  <Token size="sm" label={c.region} />
                </MetadataListItem>
              )}
              {c.note && <MetadataListItem label={t("note")}>{c.note}</MetadataListItem>}
            </MetadataList>
          </Stack>
        </Card>
        <Card padding={4}>
          <Stack gap={3}>
            <Heading level={2}>{t("playersOverTime")}</Heading>
            <Sparkline
              points={history.map((p) => ({ total: p.online, flagged: 0 }))}
              label={t("weekLabel")}
              height={96}
            />
            <Text type="supporting">{t("lastWeek")}</Text>
          </Stack>
        </Card>
      </Grid>

      <Card padding={0}>
        <Stack paddingInline={4} paddingBlock={3}>
          <Heading level={2}>{to("liveHunts")}</Heading>
        </Stack>
        {hunts.length === 0 ? (
          <EmptyState isCompact title={s?.hunts ? to("noHunts") : to("huntsUnavailable")} />
        ) : (
          <Table<HuntRow>
            data={hunts}
            idKey="id"
            density="compact"
            columns={[
              { key: "title", header: to("hunt.title"), width: proportional(2) },
              { key: "map", header: to("hunt.map"), width: proportional(1) },
              {
                key: "players",
                header: to("hunt.players"),
                width: proportional(2),
                renderCell: (h) => (
                  <Stack direction="horizontal" gap={1} wrap="wrap">
                    {h.players.map((p) => (
                      <Token
                        key={p.userId}
                        size="sm"
                        label={p.username ?? p.userId}
                        href={
                          p.username
                            ? `/players/${encodeURIComponent(p.username)}?c=${c.id}`
                            : undefined
                        }
                      />
                    ))}
                  </Stack>
                ),
              },
              {
                key: "count",
                header: to("hunt.slots"),
                width: pixel(90),
                align: "end",
                renderCell: (h) => `${h.playerCount}/${h.maxPlayers}`,
              },
              {
                key: "startedAt",
                header: to("hunt.started"),
                width: pixel(120),
                renderCell: (h) =>
                  h.startedAt ? <Timestamp value={h.startedAt} format="relative_short" /> : "—",
              },
            ]}
          />
        )}
      </Card>

      {c.state !== "pending" && <PerformanceSection perf={perf} single />}

      {audit && (
        <Card padding={0}>
          <Stack paddingInline={4} paddingBlock={3}>
            <Heading level={2}>{t("activity")}</Heading>
          </Stack>
          <AuditTable events={audit} canRevert={canRevert} />
        </Card>
      )}

      <Dialog isOpen={editing} onOpenChange={setEditing} purpose="form" width={480}>
        <Stack gap={4}>
          <DialogHeader title={t("editTitle")} onOpenChange={setEditing} />
          <Stack gap={4} paddingInline={4}>
            <TextInput label={t("name")} value={name} onChange={setName} isRequired />
            <TextInput label={t("region")} value={region} onChange={setRegion} isOptional />
            <TextArea
              label={t("note")}
              value={note}
              onChange={setNote}
              isOptional
              rows={3}
              maxLength={300}
            />
          </Stack>
          <Stack direction="horizontal" gap={2} justify="end" paddingInline={4} paddingBlockEnd={4}>
            <Button label={tc("cancel")} variant="ghost" onClick={() => setEditing(false)} />
            <Button
              label={tc("save")}
              variant="primary"
              isDisabled={!name.trim()}
              clickAction={async () => {
                if (
                  (await run(
                    () =>
                      updateControllerAction(c.id, {
                        name,
                        region: region || null,
                        note: note || null,
                      }),
                    t("saved"),
                  )) !== false
                )
                  setEditing(false);
              }}
            />
          </Stack>
        </Stack>
      </Dialog>

      <Dialog isOpen={code !== null} onOpenChange={(open) => !open && setCode(null)} width={560}>
        <Stack gap={4}>
          <DialogHeader
            title={t("pairing.title", { name: c.name })}
            onOpenChange={() => setCode(null)}
          />
          <Stack paddingInline={4} paddingBlockEnd={4}>
            {code && (
              <PairingInstructions
                code={code}
                expiresAt={Date.now() + 30 * 60_000}
                dashboardUrl={dashboardUrl}
                insecure={insecure}
              />
            )}
          </Stack>
        </Stack>
      </Dialog>

      <AlertDialog
        isOpen={removing}
        onOpenChange={setRemoving}
        title={t("removeTitle", { name: c.name })}
        description={t("removeBody")}
        actionLabel={t("remove")}
        onAction={async () => {
          if ((await run(() => removeControllerAction(c.id), t("removed"))) !== false)
            router.push("/controllers");
        }}
      />
    </Page>
  );
}
