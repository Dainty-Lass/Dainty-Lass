"use client";

import { Card } from "@astryxdesign/core/Card";
import { Center } from "@astryxdesign/core/Center";
import { Grid } from "@astryxdesign/core/Grid";
import { Heading } from "@astryxdesign/core/Heading";
import { List, ListItem } from "@astryxdesign/core/List";
import { Spinner } from "@astryxdesign/core/Spinner";
import { Stack } from "@astryxdesign/core/Stack";
import { StatusDot } from "@astryxdesign/core/StatusDot";
import { Text } from "@astryxdesign/core/Text";
import { Token } from "@astryxdesign/core/Token";
import type { NewsItem, Snapshot } from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { useFormatter, useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { LauncherShell } from "@/components/LauncherShell";
import {
  ConnectStep,
  JoinStep,
  LocateStep,
  ReadyStep,
  RegisterStep,
  RuntimeStep,
  TaskStep,
} from "@/components/Steps";

function Step({ snapshot }: { snapshot: Snapshot }) {
  const { progress } = useBridge();
  switch (snapshot.phase) {
    case "loading":
      return <Spinner size="lg" />;
    case "join":
      return <JoinStep />;
    case "connect":
      return <ConnectStep snapshot={snapshot} />;
    case "register":
      return <RegisterStep />;
    case "locate":
      return <LocateStep snapshot={snapshot} />;
    case "runtime":
      return <RuntimeStep snapshot={snapshot} />;
    case "installing":
      return <TaskStep task={progress ?? snapshot.task} />;
    default:
      return snapshot.task ? (
        <TaskStep task={progress ?? snapshot.task} />
      ) : (
        <ReadyStep snapshot={snapshot} />
      );
  }
}

/** The server as the launcher sees it: on the art, its name, who is on, and what is running. */
function ServerHero({ snapshot }: { snapshot: Snapshot }) {
  const t = useTranslations("home");
  const s = snapshot.status;
  const hide = snapshot.settings.streamerMode;
  return (
    <section className="dainty-hero">
      <Stack gap={3} padding={6}>
        <Stack direction="horizontal" gap={2} align="center">
          <StatusDot
            variant={s?.online ? "success" : "neutral"}
            label={s?.online ? t("online") : t("offline")}
            isPulsing={Boolean(s?.online)}
          />
          <Text type="label" color="secondary">
            {s?.online
              ? t("serverOnline", { count: s.playersOnline })
              : snapshot.server
                ? t("serverOffline")
                : t("noServer")}
          </Text>
        </Stack>
        <Heading level={1} type="display-2">
          {snapshot.server?.name ?? t("welcome")}
        </Heading>
        {s && (
          <Stack direction="horizontal" gap={1} wrap="wrap">
            <Token size="sm" label={t("version", { version: s.version })} />
            {s.game && <Token size="sm" label={t("build", { build: s.game.build })} />}
            <Token
              size="sm"
              label={t(`gameData.${s.gameData}`)}
              color={s.gameData === "real" ? "green" : "orange"}
            />
            {!hide && snapshot.server?.mode && (
              <Token size="sm" label={t(`modes.${snapshot.server.mode}`)} />
            )}
          </Stack>
        )}
      </Stack>
    </section>
  );
}

export default function Home() {
  const t = useTranslations("home");
  const format = useFormatter();
  const { snapshot, api } = useBridge();
  const [news, setNews] = useState<NewsItem[]>([]);

  // Status is polled by the backend only while someone is looking at it.
  useEffect(() => {
    if (!api) return;
    void api.setStatusPolling(true);
    void api.getNews().then(setNews, () => setNews([]));
    return () => void api.setStatusPolling(false);
  }, [api]);

  if (!snapshot) {
    return (
      <Center height="100dvh">
        <Spinner size="lg" />
      </Center>
    );
  }

  const s = snapshot.status;
  const hide = snapshot.settings.streamerMode;

  return (
    <LauncherShell>
      <Stack gap={4} padding={6} isScrollable height="100%">
        <ServerHero snapshot={snapshot} />
        <Card padding={5} elevation="low">
          <Step snapshot={snapshot} />
        </Card>
        <Grid columns={{ minWidth: 300, repeat: "fit" }} gap={4}>
          <Card padding={4}>
            <Stack gap={3}>
              <Heading level={2}>{t("news")}</Heading>
              {news.length === 0 ? (
                <Text type="supporting">{t("noNews")}</Text>
              ) : (
                news.slice(0, 5).map((n) => (
                  <Stack key={`${n.date}-${n.title}`} gap={0.5}>
                    <Stack direction="horizontal" gap={2} align="center">
                      <Text weight="semibold">{n.title}</Text>
                      {n.date && (
                        <Text type="supporting">
                          {format.dateTime(new Date(n.date), { dateStyle: "medium" })}
                        </Text>
                      )}
                    </Stack>
                    <Text type="supporting">{n.body}</Text>
                  </Stack>
                ))
              )}
            </Stack>
          </Card>
          <Card padding={4}>
            <Stack gap={3}>
              <Heading level={2}>{t("whoIsOn")}</Heading>
              {!s || s.limited ? (
                <Text type="supporting">{s?.limited ? t("limited") : t("noStatus")}</Text>
              ) : s.instances.length === 0 ? (
                <Text type="supporting">{t("nobody")}</Text>
              ) : (
                <List density="compact">
                  {s.instances.map((i) => (
                    <ListItem
                      key={i.id}
                      label={i.title}
                      description={
                        hide
                          ? undefined
                          : s.players
                              .filter((p) => p.instance === i.id)
                              .map((p) => p.name)
                              .join(", ") || undefined
                      }
                      endContent={<Text type="supporting">{`${i.players}/${i.maxPlayers}`}</Text>}
                    />
                  ))}
                </List>
              )}
            </Stack>
          </Card>
        </Grid>
      </Stack>
    </LauncherShell>
  );
}
