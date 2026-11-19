"use client";

import { Badge } from "@astryxdesign/core/Badge";
import { Card } from "@astryxdesign/core/Card";
import { Heading } from "@astryxdesign/core/Heading";
import { ProgressBar } from "@astryxdesign/core/ProgressBar";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { useBridge } from "@dainty/bridge/react";
import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { BADGE_HOURS, hoursOf, nextBadge, unlockedBadges } from "./playtime";

/** The signed-in account's hours online and the badges they unlocked. */
export function PlaytimeCard() {
  const t = useTranslations("launcher.playtime");
  const { snapshot, api } = useBridge();
  // undefined while asking, null when the server gives none
  const [seconds, setSeconds] = useState<number | null | undefined>(undefined);
  const server = snapshot?.server?.name;
  const hasKey = snapshot?.account.hasKey === true;

  useEffect(() => {
    if (!api || !hasKey || server === undefined) {
      setSeconds(null);
      return;
    }
    let live = true;
    setSeconds(undefined);
    api
      .getPlaytime()
      .then((s) => live && setSeconds(s))
      .catch(() => live && setSeconds(null));
    return () => {
      live = false;
    };
  }, [api, hasKey, server]);

  if (seconds === undefined) return null;
  const unlocked = seconds === null ? [] : unlockedBadges(seconds);
  const next = seconds === null ? null : nextBadge(seconds);

  return (
    <Card padding={5}>
      <Stack gap={4}>
        <Heading level={2}>{t("title")}</Heading>
        {seconds === null ? (
          <Text type="supporting">{t("unavailable")}</Text>
        ) : (
          <>
            <Text weight="semibold">{t("hours", { hours: hoursOf(seconds) })}</Text>
            {next && (
              <ProgressBar
                label={t("next", { hours: next.hoursLeft, badge: t(`badges.h${next.mark}`) })}
                value={Math.min(100, (seconds / (next.mark * 3600)) * 100)}
              />
            )}
            <Stack direction="horizontal" gap={2} wrap="wrap">
              {BADGE_HOURS.map((mark) =>
                unlocked.includes(mark) ? (
                  <Badge key={mark} variant="success" label={t(`badges.h${mark}`)} />
                ) : (
                  <Badge key={mark} label={t("locked", { hours: mark })} />
                ),
              )}
            </Stack>
          </>
        )}
      </Stack>
    </Card>
  );
}
