"use client";

import { Badge } from "@astryxdesign/core/Badge";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Grid } from "@astryxdesign/core/Grid";
import { Heading } from "@astryxdesign/core/Heading";
import { Link } from "@astryxdesign/core/Link";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Token } from "@astryxdesign/core/Token";
import type {
  ComponentMetrics,
  ComponentPhase,
  GameServerMetrics,
  MetricsSummary,
  ProcessMetrics,
} from "@dainty/fleet";
import { useFormatter, useTranslations } from "next-intl";
import type { ReactNode } from "react";
import { ControllerState } from "@/components/ControllerState";
import { Sparkline } from "@/components/Sparkline";
import type { ControllerPerformance, Performance, RouteRate } from "@/lib/fleet/metrics";

const PHASE_DOT: Record<ComponentPhase, "success" | "warning" | "error" | "neutral"> = {
  ready: "success",
  starting: "warning",
  stopping: "warning",
  failed: "error",
  stopped: "error",
  disabled: "neutral",
  unknown: "neutral",
};
/** Problems first, as everywhere else. */
const PHASE_ORDER: Record<ComponentPhase, number> = {
  failed: 0,
  stopped: 1,
  starting: 2,
  stopping: 3,
  unknown: 4,
  ready: 5,
  disabled: 6,
};
const VERDICT_BADGE: Record<string, "success" | "warning" | "orange" | "error"> = {
  allowed: "success",
  flagged: "warning",
  masked: "orange",
  blocked: "error",
};
const VERDICT_ORDER = ["allowed", "flagged", "masked", "blocked"] as const;
const KINDS = ["city", "hunt", "dojo", "tutorial"] as const;
const POSTS = ["ok", "refused", "failed"] as const;

/** A label from the catalog when the game sent a value it has one for, else the value itself. */
function known<T extends string>(values: readonly T[], value: string): T | null {
  return (values as readonly string[]).includes(value) ? (value as T) : null;
}
const POST_BADGE: Record<string, "success" | "warning" | "error"> = {
  ok: "success",
  refused: "warning",
  failed: "error",
};

type Row<T> = T & Record<string, unknown>;
type ComponentRow = Row<ComponentMetrics & { process: ProcessMetrics | null }>;
type ControllerRow = Row<ControllerPerformance>;

function useUnits() {
  const t = useTranslations("performance.units");
  const format = useFormatter();
  return {
    bytes(n: number) {
      const gb = n >= 1024 ** 3;
      return format.number(n / 1024 ** (gb ? 3 : 2), {
        style: "unit",
        unit: gb ? "gigabyte" : "megabyte",
        maximumFractionDigits: gb ? 1 : 0,
      });
    },
    cpu: (percent: number) =>
      format.number(percent / 100, { style: "percent", maximumFractionDigits: 1 }),
    ms: (v: number | null) =>
      v === null
        ? "—"
        : t("ms", { value: format.number(v, { maximumFractionDigits: v < 10 ? 1 : 0 }) }),
    perSecond: (v: number | null) =>
      v === null
        ? "—"
        : t("perSecond", { value: format.number(v, { maximumFractionDigits: v < 10 ? 2 : 0 }) }),
    perMinute: (v: number) => t("perMinute", { value: format.number(v) }),
    share: (v: number | null) =>
      v === null ? "—" : format.number(v, { style: "percent", maximumFractionDigits: 1 }),
    uptime(seconds: number) {
      const s = Math.max(0, Math.floor(seconds));
      if (s >= 86_400)
        return t("days", { days: Math.floor(s / 86_400), hours: Math.floor((s % 86_400) / 3600) });
      if (s >= 3600)
        return t("hours", { hours: Math.floor(s / 3600), minutes: Math.floor((s % 3600) / 60) });
      if (s >= 60) return t("minutes", { minutes: Math.floor(s / 60) });
      return t("seconds", { seconds: s });
    },
  };
}

/** The process a component runs as, when one is named after it (mongod, turn-rs); most run in the host. */
function processFor(component: string, processes: ProcessMetrics[]): ProcessMetrics | null {
  const want = component.toLowerCase();
  return (
    processes.find((p) => {
      if (p.role !== "child") return false;
      const name = p.name.toLowerCase().replace(/\.exe$/, "");
      return name === want || name.startsWith(`${want}-`) || want.startsWith(`${name}-`);
    }) ?? null
  );
}

function CardHeader({ title, aside }: { title: string; aside?: ReactNode }) {
  return (
    <Stack
      direction="horizontal"
      justify="between"
      align="center"
      paddingInline={4}
      paddingBlock={3}
      gap={2}
      wrap="wrap"
    >
      <Heading level={3}>{title}</Heading>
      {aside && <Text type="supporting">{aside}</Text>}
    </Stack>
  );
}

function Trends({ perf }: { perf: Performance }) {
  const t = useTranslations("performance");
  const u = useUnits();
  const last = perf.series.findLast((p) => p.cpu > 0 || p.memory > 0 || p.requests > 0);
  const charts = [
    {
      key: "requests",
      title: t("requests"),
      value: u.perMinute(last?.requests ?? 0),
      label: t("requestsLabel"),
      points: perf.series.map((p) => ({ total: p.requests, flagged: p.errors })),
    },
    {
      key: "cpu",
      title: t("cpu"),
      value: u.cpu(last?.cpu ?? 0),
      label: t("cpuLabel"),
      points: perf.series.map((p) => ({ total: p.cpu, flagged: 0 })),
    },
    {
      key: "memory",
      title: t("memory"),
      value: u.bytes(last?.memory ?? 0),
      label: t("memoryLabel"),
      points: perf.series.map((p) => ({ total: p.memory, flagged: 0 })),
    },
  ];
  return (
    <Grid columns={{ minWidth: 260, repeat: "fit" }} gap={4}>
      {charts.map((c) => (
        <Card key={c.key} padding={4}>
          <Stack gap={2}>
            <Stack direction="horizontal" justify="between" align="center">
              <Heading level={3}>{c.title}</Heading>
              <Text weight="semibold">{c.value}</Text>
            </Stack>
            <Sparkline points={c.points} label={c.label} height={48} />
            <Text type="supporting">{t("lastHour")}</Text>
          </Stack>
        </Card>
      ))}
    </Grid>
  );
}

function ComponentsCard({ summary }: { summary: MetricsSummary }) {
  const t = useTranslations("performance");
  const u = useUnits();
  const rows = summary.components
    .map((c) => ({ ...c, process: processFor(c.name, summary.processes) }))
    .sort((a, b) => PHASE_ORDER[a.phase] - PHASE_ORDER[b.phase] || a.name.localeCompare(b.name));
  return (
    <Card padding={0}>
      <CardHeader title={t("components")} aside={t("componentsHint")} />
      <Table<ComponentRow>
        data={rows as ComponentRow[]}
        idKey="name"
        density="compact"
        columns={[
          {
            key: "name",
            header: t("columns.component"),
            width: proportional(2),
            renderCell: (c) => <Text weight="medium">{c.name}</Text>,
          },
          {
            key: "phase",
            header: t("columns.phase"),
            width: pixel(130),
            renderCell: (c) => (
              <Stack direction="horizontal" gap={1.5} align="center">
                <StatusDot variant={PHASE_DOT[c.phase]} label={t(`phases.${c.phase}`)} />
                <Text type="supporting">{t(`phases.${c.phase}`)}</Text>
              </Stack>
            ),
          },
          {
            key: "uptimeSeconds",
            header: t("columns.uptime"),
            width: pixel(120),
            align: "end",
            renderCell: (c) => (c.phase === "ready" ? u.uptime(c.uptimeSeconds) : "—"),
          },
          {
            key: "restarts",
            header: t("columns.restarts"),
            width: pixel(90),
            align: "end",
            renderCell: (c) =>
              c.restarts > 0 ? <Badge variant="warning" label={c.restarts} /> : "0",
          },
          {
            key: "cpu",
            header: t("columns.cpu"),
            width: pixel(90),
            align: "end",
            renderCell: (c) => (c.process ? u.cpu(c.process.cpuPercent) : "—"),
          },
          {
            key: "memory",
            header: t("columns.memory"),
            width: pixel(100),
            align: "end",
            renderCell: (c) => (c.process ? u.bytes(c.process.memoryBytes) : "—"),
          },
        ]}
      />
    </Card>
  );
}

function GameServersCard({ summary }: { summary: MetricsSummary }) {
  const t = useTranslations("performance");
  const u = useUnits();
  const kind = (k: string) => {
    const key = known(KINDS, k);
    return key ? t(`kinds.${key}`) : k;
  };
  return (
    <Card padding={0}>
      <CardHeader title={t("gameServers")} />
      {summary.gameServers.length === 0 ? (
        <EmptyState
          isCompact
          title={summary.directorUp === false ? t("directorDown") : t("noGameServers")}
        />
      ) : (
        <Table<Row<GameServerMetrics>>
          data={summary.gameServers as Row<GameServerMetrics>[]}
          idKey="instance"
          density="compact"
          columns={[
            {
              key: "instance",
              header: t("columns.instance"),
              width: proportional(2),
              renderCell: (g) => <Text type="code">{g.instance}</Text>,
            },
            {
              key: "kind",
              header: t("columns.kind"),
              width: pixel(150),
              renderCell: (g) => <Token size="sm" label={kind(g.kind)} />,
            },
            {
              key: "players",
              header: t("columns.players"),
              width: pixel(100),
              align: "end",
              renderCell: (g) => `${g.players}/${g.maxPlayers}`,
            },
          ]}
        />
      )}
      <CardHeader title={t("processes")} aside={t("processesHint")} />
      <Table<Row<ProcessMetrics>>
        data={summary.processes as Row<ProcessMetrics>[]}
        idKey={(p) => String(p.pid)}
        density="compact"
        columns={[
          {
            key: "name",
            header: t("columns.process"),
            width: proportional(2),
            renderCell: (p) => <Text weight="medium">{p.name}</Text>,
          },
          {
            key: "role",
            header: t("columns.role"),
            width: pixel(140),
            renderCell: (p) => (
              <Token
                size="sm"
                label={t(`roles.${p.role}`)}
                color={p.role === "host" ? "blue" : undefined}
              />
            ),
          },
          { key: "pid", header: t("columns.pid"), width: pixel(80), align: "end" },
          {
            key: "cpuPercent",
            header: t("columns.cpu"),
            width: pixel(90),
            align: "end",
            renderCell: (p) => u.cpu(p.cpuPercent),
          },
          {
            key: "memoryBytes",
            header: t("columns.memory"),
            width: pixel(100),
            align: "end",
            renderCell: (p) => u.bytes(p.memoryBytes),
          },
          {
            key: "uptimeSeconds",
            header: t("columns.uptime"),
            width: pixel(120),
            align: "end",
            renderCell: (p) => u.uptime(p.uptimeSeconds),
          },
        ]}
      />
    </Card>
  );
}

function ControllersCard({ perf }: { perf: Performance }) {
  const t = useTranslations("performance");
  const u = useUnits();
  const sorted = [...perf.controllers].sort((a, b) => {
    const bad = (c: ControllerPerformance) =>
      c.summary
        ? c.summary.components.filter((x) => x.phase !== "ready" && x.phase !== "disabled").length
        : -1;
    return bad(b) - bad(a) || a.name.localeCompare(b.name);
  });
  return (
    <Card padding={0}>
      <CardHeader title={t("byController")} />
      <Table<ControllerRow>
        data={sorted as ControllerRow[]}
        idKey="id"
        density="compact"
        columns={[
          {
            key: "name",
            header: t("columns.controller"),
            width: proportional(2),
            renderCell: (c) => (
              <Stack direction="horizontal" gap={2} align="center">
                <ControllerState state={c.state} withText={false} />
                <Link href={`/controllers/${c.id}`}>{c.name}</Link>
              </Stack>
            ),
          },
          {
            key: "ready",
            header: t("columns.ready"),
            width: pixel(100),
            align: "end",
            renderCell: (c) => {
              if (!c.summary) return "—";
              const counted = c.summary.components.filter((x) => x.phase !== "disabled");
              const ready = counted.filter((x) => x.phase === "ready").length;
              return ready < counted.length ? (
                <Badge variant="error" label={`${ready}/${counted.length}`} />
              ) : (
                `${ready}/${counted.length}`
              );
            },
          },
          {
            key: "restarts",
            header: t("columns.restarts"),
            width: pixel(90),
            align: "end",
            renderCell: (c) =>
              c.summary ? String(c.summary.components.reduce((n, x) => n + x.restarts, 0)) : "—",
          },
          {
            key: "players",
            header: t("gameServers"),
            width: pixel(120),
            align: "end",
            renderCell: (c) => (c.summary ? String(c.summary.gameServers.length) : "—"),
          },
          {
            key: "cpu",
            header: t("columns.cpu"),
            width: pixel(90),
            align: "end",
            renderCell: (c) =>
              c.summary ? u.cpu(c.summary.processes.reduce((n, p) => n + p.cpuPercent, 0)) : "—",
          },
          {
            key: "memory",
            header: t("columns.memory"),
            width: pixel(100),
            align: "end",
            renderCell: (c) =>
              c.summary ? u.bytes(c.summary.processes.reduce((n, p) => n + p.memoryBytes, 0)) : "—",
          },
          {
            key: "rate",
            header: t("columns.rate"),
            width: pixel(100),
            align: "end",
            renderCell: (c) => u.perSecond(c.overall?.perSecond ?? null),
          },
          {
            key: "p95",
            header: t("columns.p95"),
            width: pixel(90),
            align: "end",
            renderCell: (c) => u.ms(c.overall?.p95 ?? null),
          },
        ]}
      />
    </Card>
  );
}

function UpstreamsCard({ routes }: { routes: RouteRate[] }) {
  const t = useTranslations("performance");
  const u = useUnits();
  return (
    <Card padding={0}>
      <CardHeader title={t("upstreams")} aside={t("upstreamsHint")} />
      {routes.length === 0 ? (
        <EmptyState isCompact title={t("noRequests")} />
      ) : (
        <Table<Row<RouteRate>>
          data={routes as Row<RouteRate>[]}
          idKey={(r) => `${r.service}/${r.route}`}
          density="compact"
          columns={[
            {
              key: "service",
              header: t("columns.service"),
              width: proportional(1),
              renderCell: (r) => <Text weight="medium">{r.service}</Text>,
            },
            {
              key: "route",
              header: t("columns.route"),
              width: proportional(1),
              renderCell: (r) => <Token size="sm" label={r.route} />,
            },
            {
              key: "perSecond",
              header: t("columns.rate"),
              width: pixel(100),
              align: "end",
              renderCell: (r) => u.perSecond(r.perSecond),
            },
            {
              key: "errorShare",
              header: t("columns.errors"),
              width: pixel(80),
              align: "end",
              renderCell: (r) =>
                r.errorShare ? (
                  <Badge variant="error" label={u.share(r.errorShare)} />
                ) : (
                  u.share(r.errorShare)
                ),
            },
            {
              key: "p50",
              header: t("columns.p50"),
              width: pixel(90),
              align: "end",
              renderCell: (r) => u.ms(r.p50),
            },
            {
              key: "p95",
              header: t("columns.p95"),
              width: pixel(90),
              align: "end",
              renderCell: (r) => u.ms(r.p95),
            },
          ]}
        />
      )}
    </Card>
  );
}

function ChatCard({
  chat,
  since,
  posts,
}: {
  chat: Record<string, number>;
  since: number | null;
  posts: Record<string, number> | null;
}) {
  const t = useTranslations("performance");
  const tf = useTranslations("filter.verdicts");
  const format = useFormatter();
  const verdicts = [
    ...VERDICT_ORDER.filter((v) => v in chat),
    ...Object.keys(chat)
      .filter((v) => !known(VERDICT_ORDER, v))
      .sort(),
  ];
  const verdictLabel = (v: string) => {
    const key = known(VERDICT_ORDER, v);
    return key ? tf(key) : v;
  };
  const postLabel = (r: string) => {
    const key = known(POSTS, r);
    return key ? t(`posts.${key}`) : r;
  };
  return (
    <Card padding={0}>
      <CardHeader
        title={t("chat")}
        aside={
          since ? (
            <>
              {t("chatSince")}{" "}
              <Timestamp value={new Date(since).toISOString()} format="relative" isLive />
            </>
          ) : undefined
        }
      />
      {verdicts.every((v) => !chat[v]) ? (
        <EmptyState isCompact title={t("noChat")} />
      ) : (
        <Table<Row<{ verdict: string; count: number }>>
          data={verdicts.map((verdict) => ({ verdict, count: chat[verdict] ?? 0 }))}
          idKey="verdict"
          density="compact"
          columns={[
            {
              key: "verdict",
              header: t("columns.verdict"),
              width: proportional(1),
              renderCell: (r) => (
                <Badge
                  variant={VERDICT_BADGE[r.verdict] ?? "neutral"}
                  label={verdictLabel(r.verdict)}
                />
              ),
            },
            {
              key: "count",
              header: t("columns.count"),
              width: pixel(100),
              align: "end",
              renderCell: (r) => format.number(r.count),
            },
          ]}
        />
      )}
      {posts && Object.keys(posts).length > 0 && (
        <>
          <CardHeader title={t("dashboardPosts")} aside={t("sinceStart")} />
          <Table<Row<{ result: string; count: number }>>
            data={Object.entries(posts).map(([result, count]) => ({ result, count }))}
            idKey="result"
            density="compact"
            columns={[
              {
                key: "result",
                header: t("columns.result"),
                width: proportional(1),
                renderCell: (r) => (
                  <Badge variant={POST_BADGE[r.result] ?? "neutral"} label={postLabel(r.result)} />
                ),
              },
              {
                key: "count",
                header: t("columns.posts"),
                width: pixel(100),
                align: "end",
                renderCell: (r) => format.number(r.count),
              },
            ]}
          />
        </>
      )}
    </Card>
  );
}

/**
 * Every component's health and load, from the controllers' `/metrics` (lib/fleet/metrics.ts). One
 * controller shows its components, game servers and processes; a fleet shows a row per controller.
 * `perf.chat` is null where the reader may not see chat.
 */
export function PerformanceSection({ perf, single }: { perf: Performance; single: boolean }) {
  const t = useTranslations("performance");
  const one = single && perf.controllers.length === 1 ? perf.controllers[0]! : null;
  const reported = perf.controllers.filter((c) => c.summary);
  const latest = Math.max(0, ...reported.map((c) => c.summary!.at));
  const posts = one?.summary?.dashboardPosts ?? null;

  return (
    <Stack gap={4}>
      <Stack direction="horizontal" justify="between" align="end" gap={2} wrap="wrap">
        <Heading level={2}>{t("title")}</Heading>
        {latest > 0 && (
          <Text type="supporting">
            {t("updated")}{" "}
            <Timestamp value={new Date(latest).toISOString()} format="relative" isLive />
          </Text>
        )}
      </Stack>
      {reported.length === 0 ? (
        <Card padding={4}>
          <EmptyState isCompact title={t("none")} description={t("noneHint")} />
        </Card>
      ) : (
        <>
          <Trends perf={perf} />
          {one?.summary ? (
            <>
              <ComponentsCard summary={one.summary} />
              <GameServersCard summary={one.summary} />
            </>
          ) : (
            <ControllersCard perf={perf} />
          )}
          <UpstreamsCard routes={one ? one.routes : perf.routes} />
          {perf.chat && <ChatCard chat={perf.chat} since={perf.chatSinceMs} posts={posts} />}
        </>
      )}
    </Stack>
  );
}
