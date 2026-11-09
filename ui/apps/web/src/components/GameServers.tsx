"use client";

import { Badge } from "@astryxdesign/core/Badge";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Heading } from "@astryxdesign/core/Heading";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Token } from "@astryxdesign/core/Token";
import { useTranslations } from "next-intl";
import type { Availability, GameEvent, GameServer, GamesReport, Install } from "@/lib/fleet/games";

const REASONS = new Set(["director_unreachable", "no_hub", "hub_not_ready", "module_not_ready"]);
const INSTALL_DOT = {
  online: "success",
  degraded: "warning",
  offline: "error",
  revoked: "neutral",
} as const;

type ServerRow = GameServer & Record<string, unknown>;
type EventRow = GameEvent & Record<string, unknown>;
type InstallRow = Install & Record<string, unknown>;

/** Up, or the translated reason it is not; a code the catalog lacks is shown as the code. */
export function AvailabilityState({ availability }: { availability: Availability | null }) {
  const t = useTranslations("games");
  if (!availability) return <StatusDot variant="neutral" label={t("availabilityUnknown")} />;
  const label = availability.up
    ? t("up")
    : availability.reason && REASONS.has(availability.reason)
      ? t(`reasons.${availability.reason}` as "reasons.no_hub")
      : t("reasons.other", { code: availability.reason ?? "" });
  return (
    <Stack direction="horizontal" gap={1.5} align="center">
      <StatusDot variant={availability.up ? "success" : "error"} label={label} />
      <Text type="supporting">{label}</Text>
    </Stack>
  );
}

function ServerTable({ servers }: { servers: GameServer[] }) {
  const t = useTranslations("games");
  return (
    <Table<ServerRow>
      data={servers as ServerRow[]}
      idKey={(s) => `${s.game}:${s.id}`}
      density="compact"
      columns={[
        {
          key: "title",
          header: t("columns.title"),
          width: proportional(2),
          renderCell: (s) => <Text weight="medium">{s.title}</Text>,
        },
        {
          key: "kind",
          header: t("columns.kind"),
          width: pixel(100),
          renderCell: (s) => t(`kinds.${s.kind}`),
        },
        { key: "map", header: t("columns.map"), width: proportional(1) },
        {
          key: "port",
          header: t("columns.address"),
          width: proportional(1),
          renderCell: (s) => t("address", { host: s.host, port: s.port }),
        },
        {
          key: "players",
          header: t("columns.players"),
          width: pixel(90),
          align: "end",
          renderCell: (s) => t("slots", { players: s.players, max: s.maxPlayers }),
        },
        {
          key: "ready",
          header: t("columns.ready"),
          width: pixel(100),
          renderCell: (s) => (
            <Badge
              variant={s.ready ? "success" : "neutral"}
              label={s.ready ? t("ready") : t("starting")}
            />
          ),
        },
        {
          key: "startedAt",
          header: t("columns.started"),
          width: pixel(120),
          renderCell: (s) =>
            s.startedAt ? <Timestamp value={s.startedAt} format="relative_short" /> : "—",
        },
      ]}
    />
  );
}

/** This server's game servers and whether players can get in, from the last report. */
export function GameServersCard({ report }: { report: GamesReport | null }) {
  const t = useTranslations("games");
  const servers = report?.gameServers ?? null;
  const free = report?.availability?.freeHunts;
  return (
    <Card padding={0}>
      <Stack gap={0}>
        <Stack
          direction="horizontal"
          justify="between"
          align="center"
          paddingInline={4}
          paddingBlock={3}
        >
          <Heading level={2}>{t("title")}</Heading>
          <Stack direction="horizontal" gap={3} align="center">
            {free != null && <Text type="supporting">{t("freeHunts", { count: free })}</Text>}
            {report && <AvailabilityState availability={report.availability} />}
          </Stack>
        </Stack>
        {!report ? (
          <EmptyState isCompact title={t("waiting")} />
        ) : servers === null ? (
          <EmptyState isCompact title={t("unreachable")} />
        ) : servers.length === 0 ? (
          <EmptyState isCompact title={t("none")} />
        ) : (
          <ServerTable servers={servers} />
        )}
      </Stack>
    </Card>
  );
}

/** What the server's event recorder kept, newest first. */
export function GameEventsCard({ events }: { events: GameEvent[] }) {
  const t = useTranslations("games.events");
  return (
    <Card padding={0}>
      <Stack gap={0}>
        <Stack paddingInline={4} paddingBlock={3}>
          <Heading level={2}>{t("title")}</Heading>
        </Stack>
        {events.length === 0 ? (
          <EmptyState isCompact title={t("none")} />
        ) : (
          <Table<EventRow>
            data={events as EventRow[]}
            idKey="seq"
            density="compact"
            columns={[
              {
                key: "name",
                header: t("name"),
                width: proportional(2),
                renderCell: (e) => <Text weight="medium">{e.name}</Text>,
              },
              {
                key: "source",
                header: t("source"),
                width: proportional(1),
                renderCell: (e) => e.source ?? t("host"),
              },
              {
                key: "atMs",
                header: t("at"),
                width: pixel(140),
                renderCell: (e) => (
                  <Timestamp value={new Date(e.atMs).toISOString()} format="relative_short" />
                ),
              },
            ]}
          />
        )}
      </Stack>
    </Card>
  );
}

/** Every install the remote controller beside this dashboard manages (D1). */
export function InstallsCard({ report }: { report: GamesReport | null }) {
  const t = useTranslations("games.installs");
  const tg = useTranslations("games");
  const installs = report?.installs ?? null;
  return (
    <Card padding={0}>
      <Stack gap={0}>
        <Stack gap={1} paddingInline={4} paddingBlock={3}>
          <Heading level={2}>{t("title")}</Heading>
          <Text type="supporting">{t("hint")}</Text>
        </Stack>
        {!report ? (
          <EmptyState isCompact title={tg("waiting")} />
        ) : installs === null ? (
          <EmptyState isCompact title={t("notController")} />
        ) : installs.length === 0 ? (
          <EmptyState isCompact title={t("none")} />
        ) : (
          <Table<InstallRow>
            data={installs as InstallRow[]}
            idKey="node"
            density="compact"
            columns={[
              {
                key: "label",
                header: t("columns.label"),
                width: proportional(2),
                renderCell: (i) => (
                  <Stack gap={0.5}>
                    <Text weight="medium">{i.label}</Text>
                    <Text type="supporting">{i.node}</Text>
                  </Stack>
                ),
              },
              { key: "region", header: t("columns.region"), width: proportional(1) },
              {
                key: "state",
                header: t("columns.state"),
                width: pixel(120),
                renderCell: (i) => (
                  <Stack direction="horizontal" gap={1.5} align="center">
                    <StatusDot
                      variant={INSTALL_DOT[i.state] ?? "neutral"}
                      label={t(`states.${i.state}`)}
                    />
                    <Text type="supporting">{t(`states.${i.state}`)}</Text>
                  </Stack>
                ),
              },
              {
                key: "availability",
                header: t("columns.availability"),
                width: proportional(2),
                renderCell: (i) =>
                  i.availability ? (
                    <AvailabilityState availability={i.availability} />
                  ) : (
                    <Text type="supporting">{t("noAvailability")}</Text>
                  ),
              },
              {
                key: "gameServers",
                header: t("columns.servers"),
                width: proportional(2),
                renderCell: (i) => (
                  <Stack direction="horizontal" gap={1} wrap="wrap">
                    {i.gameServers.map((s) => (
                      <Token
                        key={`${s.game}:${s.id}`}
                        size="sm"
                        label={tg("serverToken", {
                          title: s.title,
                          players: s.players,
                          max: s.maxPlayers,
                        })}
                      />
                    ))}
                  </Stack>
                ),
              },
              {
                key: "playersOnline",
                header: t("columns.players"),
                width: pixel(90),
                align: "end",
                renderCell: (i) => (i.playersOnline == null ? "—" : String(i.playersOnline)),
              },
              { key: "version", header: t("columns.version"), width: pixel(100) },
              {
                key: "lastReportAt",
                header: t("columns.lastReport"),
                width: pixel(120),
                renderCell: (i) =>
                  i.lastReportAt ? (
                    <Timestamp value={i.lastReportAt} format="relative_short" />
                  ) : (
                    t("noReport")
                  ),
              },
            ]}
          />
        )}
      </Stack>
    </Card>
  );
}
