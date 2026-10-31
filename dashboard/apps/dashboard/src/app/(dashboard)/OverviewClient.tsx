"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Grid } from "@astryxdesign/core/Grid";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Token } from "@astryxdesign/core/Token";
import { RefreshCw } from "lucide-react";
import { useRouter } from "next/navigation";
import { useFormatter, useTranslations } from "next-intl";
import { useEffect } from "react";
import { AuditSentence } from "@/components/AuditSentence";
import { Page } from "@/components/Page";
import { Sparkline } from "@/components/Sparkline";
import { StatTile } from "@/components/StatTile";
import type { AuditEvent } from "@/lib/db/schema";
import type { Health, ServiceState } from "@/lib/health";

type Hunt = NonNullable<Health["hunts"]>["hunts"][number];

const DOT: Record<ServiceState, "success" | "error" | "neutral"> = {
  ok: "success",
  down: "error",
  not_configured: "neutral",
};

export function OverviewClient({
  health,
  reports,
  chat,
  hourly,
  changesToday,
  recent,
  filterVersion,
  denied,
}: {
  health: Health;
  reports: Record<"open" | "claimed" | "resolved" | "dismissed", number>;
  chat: { total: number; flagged: number; senders: number };
  hourly: { hour: number; total: number; flagged: number }[];
  changesToday: number;
  recent: AuditEvent[];
  filterVersion: number;
  denied: boolean;
}) {
  const t = useTranslations("overview");
  const tc = useTranslations("common");
  const format = useFormatter();
  const router = useRouter();

  // Live enough for a moderator watching: the server data is re-read every 30 s.
  useEffect(() => {
    const id = setInterval(() => router.refresh(), 30_000);
    return () => clearInterval(id);
  }, [router]);

  const s = health.server;
  const h = health.hunts;
  const uptime = s ? format.number(Math.floor(s.uptimeSeconds / 3600)) : null;

  const services: { key: string; label: string; state: ServiceState; detail?: React.ReactNode }[] =
    [
      {
        key: "admin",
        label: t("services.adminApi"),
        state: health.adminApi,
        detail: s
          ? t("services.version", { version: s.version, commit: s.commit ?? "—" })
          : undefined,
      },
      { key: "backend", label: t("services.backend"), state: health.backend },
      {
        key: "ingest",
        label: t("services.ingest"),
        state: health.ingest,
        detail: health.lastChatAt ? (
          <>
            {t("services.lastChat")}{" "}
            <Timestamp value={new Date(health.lastChatAt).toISOString()} format="relative_short" />
          </>
        ) : (
          t("services.noChatYet")
        ),
      },
      { key: "discord", label: t("services.discord"), state: health.discord },
    ];

  return (
    <Page
      title={t("title")}
      description={
        s ? t("subtitle", { name: s.name, mode: tc(`modes.${s.mode}`) }) : t("subtitleOffline")
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

      <Grid columns={{ minWidth: 220, repeat: "fit" }} gap={4}>
        <StatTile
          label={t("stats.online")}
          href="/players?online=1"
          value={s ? format.number(s.onlineCount) : "—"}
          hint={s ? t("stats.uptime", { hours: uptime ?? "0" }) : t("stats.serverUnreachable")}
          status={
            <StatusDot
              variant={s ? "success" : "error"}
              label={s ? tc("online") : tc("offline")}
              isPulsing={Boolean(s)}
            />
          }
        />
        <StatTile
          label={t("stats.hunts")}
          value={h ? `${h.running}/${h.maxHuntServers}` : "—"}
          hint={h ? t("stats.huntsFree", { free: h.free }) : undefined}
        />
        <StatTile
          label={t("stats.openReports")}
          href="/reports"
          value={format.number(reports.open)}
          hint={t("stats.claimed", { count: reports.claimed })}
          status={
            reports.open > 0 ? (
              <StatusDot variant="warning" label={t("stats.needsAttention")} />
            ) : undefined
          }
        />
        <StatTile
          label={t("stats.flagged")}
          href="/chat?flagged=1&range=24h"
          value={format.number(chat.flagged)}
          hint={t("stats.messages", { total: chat.total, senders: chat.senders })}
        />
      </Grid>

      <Grid columns={{ minWidth: 380, repeat: "fit" }} gap={4}>
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
              <ListItem
                label={t("services.filter")}
                description={t("services.filterVersion", { version: filterVersion })}
                startContent={<StatusDot variant="accent" label={t("services.filter")} />}
                endContent={
                  <Button label={tc("open")} size="sm" variant="ghost" href="/chat-filter" />
                }
              />
            </List>
          </Stack>
        </Card>

        <Card padding={4}>
          <Stack gap={3}>
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
              height={96}
            />
            <Text type="supporting">{t("last24h")}</Text>
          </Stack>
        </Card>
      </Grid>

      <Card padding={0}>
        <Stack gap={0}>
          <Stack paddingInline={4} paddingBlock={3}>
            <Heading level={2}>{t("liveHunts")}</Heading>
          </Stack>
          {h && h.hunts.length > 0 ? (
            <Table<Hunt & Record<string, unknown>>
              data={h.hunts as (Hunt & Record<string, unknown>)[]}
              idKey="id"
              density="compact"
              columns={[
                {
                  key: "title",
                  header: t("hunt.title"),
                  width: proportional(2),
                  renderCell: (x) => <Text weight="medium">{x.title}</Text>,
                },
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
                            p.username ? `/players/${encodeURIComponent(p.username)}` : undefined
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
            <EmptyState isCompact title={h ? t("noHunts") : t("huntsUnavailable")} />
          )}
        </Stack>
      </Card>

      <Card padding={4}>
        <Stack gap={3}>
          <Stack direction="horizontal" justify="between" align="center">
            <Heading level={2}>{t("recentActivity")}</Heading>
            <Stack direction="horizontal" gap={2} align="center">
              <Text type="supporting">{t("changesToday", { count: changesToday })}</Text>
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
    </Page>
  );
}
