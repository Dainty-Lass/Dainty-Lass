"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Grid } from "@astryxdesign/core/Grid";
import { Heading } from "@astryxdesign/core/Heading";
import { Link } from "@astryxdesign/core/Link";
import { List, ListItem } from "@astryxdesign/core/List";
import { Pagination } from "@astryxdesign/core/Pagination";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Token } from "@astryxdesign/core/Token";
import { StatTiles } from "@dainty/theme";
import { RefreshCw } from "lucide-react";
import { useRouter } from "next/navigation";
import { useFormatter, useTranslations } from "next-intl";
import { type ReactNode, useEffect, useState } from "react";
import { AuditSentence } from "@/components/AuditSentence";
import { ControllerState, STATE_ORDER } from "@/components/ControllerState";
import { Page } from "@/components/Page";
import { Sparkline } from "@/components/Sparkline";

import type { AuditEvent } from "@/lib/db/schema";
import type { ControllerView } from "@/lib/fleet/status";
import type { FleetHunt, Overview, ServiceState } from "@/lib/health";

const DOT: Record<ServiceState, "success" | "error" | "neutral"> = {
  ok: "success",
  down: "error",
  not_configured: "neutral",
};
const HUNTS_PER_PAGE = 20;

type ControllerRow = ControllerView & Record<string, unknown>;
type HuntRow = FleetHunt & Record<string, unknown>;

export function OverviewClient({
  data,
  scopeName,
  reports,
  chat,
  hourly,
  changesToday,
  recent,
  filterVersion,
  denied,
}: {
  data: Overview;
  /** The one controller in scope, or null for the whole fleet. */
  scopeName: string | null;
  // Null where the reader's role may not see it: reports and chat from moderator, changes from admin.
  reports: Record<"open" | "claimed" | "resolved" | "dismissed", number> | null;
  chat: { total: number; flagged: number; senders: number } | null;
  hourly: { hour: number; total: number; flagged: number }[] | null;
  changesToday: number | null;
  recent: AuditEvent[] | null;
  filterVersion: number;
  denied: boolean;
}) {
  const t = useTranslations("overview");
  const tf = useTranslations("fleet");
  const tc = useTranslations("common");
  const format = useFormatter();
  const router = useRouter();
  const [huntPage, setHuntPage] = useState(1);

  // Status arrives every 15 s from each controller; re-reading it every 30 s keeps the page current.
  useEffect(() => {
    const id = setInterval(() => router.refresh(), 30_000);
    return () => clearInterval(id);
  }, [router]);

  const { totals } = data;
  const single = scopeName !== null && data.controllers.length === 1 ? data.controllers[0]! : null;
  const fleetWide = !single;
  const down = data.controllers.filter((c) => c.state === "offline" || c.state === "stale");
  const shownHunts = data.hunts.slice((huntPage - 1) * HUNTS_PER_PAGE, huntPage * HUNTS_PER_PAGE);

  const services: { key: string; label: string; state: ServiceState; detail?: ReactNode }[] = single
    ? [
        {
          key: "admin",
          label: t("services.adminApi"),
          state: single.status?.adminApi ?? "down",
          detail: single.status?.server
            ? t("services.version", {
                version: single.status.server.version,
                commit: single.status.server.commit ?? "—",
              })
            : undefined,
        },
        { key: "backend", label: t("services.backend"), state: single.status?.backend ?? "down" },
      ]
    : [];
  services.push(
    {
      key: "ingest",
      label: t("services.ingest"),
      state: data.ingest,
      detail: data.lastChatAt ? (
        <>
          {t("services.lastChat")}{" "}
          <Timestamp value={new Date(data.lastChatAt).toISOString()} format="relative_short" />
        </>
      ) : (
        t("services.noChatYet")
      ),
    },
    { key: "discord", label: t("services.discord"), state: data.discord },
  );

  return (
    <Page
      title={t("title")}
      description={
        scopeName
          ? t("subtitleOne", { name: scopeName })
          : t("subtitleFleet", { count: totals.controllers })
      }
      actions={
        <Button
          label={tc("refresh")}
          icon={<RefreshCw size={16} />}
          onClick={() => router.refresh()}
        />
      }
    >
      {denied && <Banner status="warning" title={t("denied")} collapsible={false} isDismissable />}
      {fleetWide && down.length > 0 && (
        <Banner
          status="error"
          title={tf("downBanner", { count: down.length })}
          description={
            down
              .slice(0, 5)
              .map((c) => c.name)
              .join(", ") + (down.length > 5 ? "…" : "")
          }
          collapsible={false}
          endContent={
            <Button label={tf("viewControllers")} size="sm" href="/controllers?state=down" />
          }
        />
      )}

      <StatTiles
        tiles={[
          {
            id: "players",
            label: t("stats.online"),
            href: "/players?online=1",
            value: format.number(totals.players),
            hue: "green",
            note: single?.status?.server
              ? t("stats.uptime", {
                  hours: format.number(Math.floor(single.status.server.uptimeSeconds / 3600)),
                })
              : undefined,
          },
          ...(fleetWide
            ? [
                {
                  id: "controllers",
                  label: tf("controllersOnline"),
                  href: "/controllers",
                  value: `${totals.online}/${totals.controllers}`,
                  hue: down.length ? ("red" as const) : ("cyan" as const),
                  flag: down.length
                    ? { label: tf("someDown"), variant: "error" as const }
                    : undefined,
                },
              ]
            : []),
          {
            id: "hunts",
            label: t("stats.hunts"),
            value: `${format.number(totals.huntsRunning)}/${format.number(totals.huntsMax)}`,
            hue: "purple",
            note: t("stats.huntsFree", {
              free: Math.max(0, totals.huntsMax - totals.huntsRunning),
            }),
          },
          ...(reports
            ? [
                {
                  id: "reports",
                  label: t("stats.openReports"),
                  href: "/reports",
                  value: format.number(reports.open),
                  hue: reports.open ? ("orange" as const) : ("gray" as const),
                  note: t("stats.claimed", { count: reports.claimed }),
                  flag: reports.open
                    ? { label: t("stats.needsAttention"), variant: "warning" as const }
                    : undefined,
                },
              ]
            : []),
          ...(chat
            ? [
                {
                  id: "flagged",
                  label: t("stats.flagged"),
                  href: "/chat?flagged=1&range=24h",
                  value: format.number(chat.flagged),
                  hue: chat.flagged ? ("red" as const) : ("gray" as const),
                  note: t("stats.messages", { total: chat.total, senders: chat.senders }),
                },
              ]
            : []),
        ]}
      />

      <Grid columns={{ minWidth: 380, repeat: "fit" }} gap={4}>
        {fleetWide ? (
          <Card padding={0}>
            <Stack
              direction="horizontal"
              justify="between"
              align="center"
              paddingInline={4}
              paddingBlock={3}
            >
              <Heading level={2}>{tf("controllers")}</Heading>
              <Button label={tf("viewControllers")} size="sm" variant="ghost" href="/controllers" />
            </Stack>
            <Table<ControllerRow>
              data={
                [...data.controllers]
                  .sort(
                    (a, b) =>
                      STATE_ORDER[a.state] - STATE_ORDER[b.state] || a.name.localeCompare(b.name),
                  )
                  .slice(0, 8) as ControllerRow[]
              }
              idKey="id"
              density="compact"
              columns={[
                {
                  key: "name",
                  header: tf("columns.controller"),
                  width: proportional(2),
                  renderCell: (c) => <Link href={`/controllers/${c.id}`}>{c.name}</Link>,
                },
                {
                  key: "state",
                  header: tf("columns.state"),
                  width: pixel(130),
                  renderCell: (c) => <ControllerState state={c.state} />,
                },
                {
                  key: "players",
                  header: tf("columns.players"),
                  width: pixel(90),
                  align: "end",
                  renderCell: (c) => format.number(c.status?.online ?? 0),
                },
                {
                  key: "hunts",
                  header: tf("columns.hunts"),
                  width: pixel(90),
                  align: "end",
                  renderCell: (c) =>
                    `${c.status?.hunts?.running ?? 0}/${c.status?.hunts?.maxHuntServers ?? 0}`,
                },
              ]}
            />
            <Stack padding={3}>
              <Text type="supporting">
                {t("services.filterVersion", { version: filterVersion })}
              </Text>
            </Stack>
          </Card>
        ) : null}

        <Card padding={4}>
          <Stack gap={3}>
            <Heading level={2}>{t("health")}</Heading>
            <List>
              {services.map((sv) => (
                <ListItem
                  key={sv.key}
                  label={sv.label}
                  description={sv.detail}
                  startContent={
                    <StatusDot variant={DOT[sv.state]} label={t(`state.${sv.state}`)} />
                  }
                  endContent={<Text type="supporting">{t(`state.${sv.state}`)}</Text>}
                />
              ))}
              {!fleetWide && (
                <ListItem
                  label={t("services.filter")}
                  description={t("services.filterVersion", { version: filterVersion })}
                  startContent={<StatusDot variant="accent" label={t("services.filter")} />}
                  endContent={
                    <Button label={tc("open")} size="sm" variant="ghost" href="/chat-filter" />
                  }
                />
              )}
            </List>
          </Stack>
        </Card>

        <Card padding={4}>
          <Stack gap={3}>
            {chat && hourly && (
              <>
                <Stack direction="horizontal" justify="between" align="center">
                  <Heading level={2}>{t("chatActivity")}</Heading>
                  <Stack direction="horizontal" gap={1}>
                    <Token size="sm" label={t("legend.messages")} color="blue" />
                    <Token size="sm" label={t("legend.flagged")} color="red" />
                  </Stack>
                </Stack>
                <Sparkline
                  points={hourly}
                  label={t("chatActivityLabel", { total: chat.total, flagged: chat.flagged })}
                  height={72}
                />
              </>
            )}
            <Heading level={chat ? 3 : 2}>{tf("playersOverTime")}</Heading>
            <Sparkline
              points={data.history.map((p) => ({ total: p.online, flagged: 0 }))}
              label={tf("playersOverTimeLabel")}
              height={56}
            />
            <Text type="supporting">{t("last24h")}</Text>
          </Stack>
        </Card>
      </Grid>

      <Card padding={0}>
        <Stack gap={0}>
          <Stack
            direction="horizontal"
            justify="between"
            align="center"
            paddingInline={4}
            paddingBlock={3}
          >
            <Heading level={2}>{t("liveHunts")}</Heading>
            <Text type="supporting">{tf("huntCount", { count: data.hunts.length })}</Text>
          </Stack>
          {shownHunts.length > 0 ? (
            <Table<HuntRow>
              data={shownHunts as HuntRow[]}
              idKey={(h) => `${h.controllerId}:${h.id}`}
              density="compact"
              columns={[
                {
                  key: "title",
                  header: t("hunt.title"),
                  width: proportional(2),
                  renderCell: (x) => <Text weight="medium">{x.title}</Text>,
                },
                ...(fleetWide
                  ? [
                      {
                        key: "controllerName",
                        header: tf("columns.controller"),
                        width: proportional(1),
                        renderCell: (x: HuntRow) => (
                          <Link href={`/controllers/${x.controllerId}`}>{x.controllerName}</Link>
                        ),
                      },
                    ]
                  : []),
                { key: "map", header: t("hunt.map"), width: proportional(1) },
                {
                  key: "players",
                  header: t("hunt.players"),
                  width: proportional(2),
                  renderCell: (x) => (
                    <Stack direction="horizontal" gap={1} wrap="wrap">
                      {x.players.map((p) => (
                        <Token
                          key={p.userId}
                          size="sm"
                          label={p.username ?? p.userId}
                          href={
                            p.username
                              ? `/players/${encodeURIComponent(p.username)}?c=${x.controllerId}`
                              : undefined
                          }
                        />
                      ))}
                    </Stack>
                  ),
                },
                {
                  key: "count",
                  header: t("hunt.slots"),
                  width: pixel(90),
                  align: "end",
                  renderCell: (x) => `${x.playerCount}/${x.maxPlayers}`,
                },
                {
                  key: "startedAt",
                  header: t("hunt.started"),
                  width: pixel(120),
                  renderCell: (x) =>
                    x.startedAt ? <Timestamp value={x.startedAt} format="relative_short" /> : "—",
                },
              ]}
            />
          ) : (
            <EmptyState isCompact title={totals.online ? t("noHunts") : t("huntsUnavailable")} />
          )}
          {data.hunts.length > HUNTS_PER_PAGE && (
            <Stack padding={3} align="center">
              <Pagination
                page={huntPage}
                totalItems={data.hunts.length}
                pageSize={HUNTS_PER_PAGE}
                onChange={setHuntPage}
                size="sm"
              />
            </Stack>
          )}
        </Stack>
      </Card>

      {recent && (
        <Card padding={4}>
          <Stack gap={3}>
            <Stack direction="horizontal" justify="between" align="center">
              <Heading level={2}>{t("recentActivity")}</Heading>
              <Stack direction="horizontal" gap={2} align="center">
                <Text type="supporting">{t("changesToday", { count: changesToday ?? 0 })}</Text>
                <Button label={t("openAudit")} size="sm" variant="ghost" href="/audit" />
              </Stack>
            </Stack>
            {recent.length === 0 ? (
              <EmptyState isCompact title={t("noActivity")} />
            ) : (
              <Stack gap={2}>
                {recent.map((ev) => (
                  <AuditSentence key={ev.id} event={ev} />
                ))}
              </Stack>
            )}
          </Stack>
        </Card>
      )}
    </Page>
  );
}
