"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Heading } from "@astryxdesign/core/Heading";
import { SegmentedControl, SegmentedControlItem } from "@astryxdesign/core/SegmentedControl";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import type { LogSettings } from "@dainty/fleet";
import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { useAction } from "@/components/useAction";
import { updateLogSettingsAction } from "../actions";

const FORMATS = ["text", "json"] as const;
const COLORS = ["auto", "always", "never"] as const;

/** settings.toml's `[logs]` on one controller: the console's format and color, and the Graylog collector. */
export function LogSettingsCard({
  controllerId,
  logs,
}: {
  controllerId: string;
  logs: LogSettings | null;
}) {
  const t = useTranslations("fleet.logs");
  const { run, pending } = useAction();
  const [format, setFormat] = useState<LogSettings["format"]>(logs?.format ?? "text");
  const [color, setColor] = useState<LogSettings["color"]>(logs?.color ?? "auto");
  const [gelfUrl, setGelfUrl] = useState(logs?.gelfUrl ?? "");

  useEffect(() => {
    if (!logs) return;
    setFormat(logs.format);
    setColor(logs.color);
    setGelfUrl(logs.gelfUrl ?? "");
  }, [logs]);

  const changed =
    logs !== null &&
    (format !== logs.format || color !== logs.color || gelfUrl.trim() !== (logs.gelfUrl ?? ""));

  return (
    <Card padding={5}>
      <Stack gap={4}>
        <Stack gap={1}>
          <Heading level={2}>{t("title")}</Heading>
          <Text type="supporting">{t("hint")}</Text>
        </Stack>
        {!logs ? (
          <Text type="supporting">{t("unavailable")}</Text>
        ) : (
          <>
            {logs.restartNeeded && (
              <Banner status="warning" title={t("restartNeeded")} collapsible={false} />
            )}
            <SegmentedControl
              label={t("format")}
              value={format}
              onChange={(v) => setFormat(v as LogSettings["format"])}
              layout="fill"
            >
              {FORMATS.map((f) => (
                <SegmentedControlItem key={f} value={f} label={t(`formats.${f}`)} />
              ))}
            </SegmentedControl>
            <SegmentedControl
              label={t("color")}
              value={color}
              onChange={(v) => setColor(v as LogSettings["color"])}
              layout="fill"
            >
              {COLORS.map((c) => (
                <SegmentedControlItem key={c} value={c} label={t(`colors.${c}`)} />
              ))}
            </SegmentedControl>
            <TextInput
              label={t("gelfUrl")}
              value={gelfUrl}
              onChange={setGelfUrl}
              placeholder="https://graylog.example.org:12201/gelf"
              description={t("gelfUrlHint")}
            />
            <Stack direction="horizontal" gap={2}>
              <Button
                label={t("save")}
                variant="primary"
                isDisabled={!changed || pending}
                clickAction={async () =>
                  void (await run(
                    () =>
                      updateLogSettingsAction(controllerId, {
                        format,
                        color,
                        gelfUrl: gelfUrl.trim(),
                      }),
                    t("saved"),
                  ))
                }
              />
            </Stack>
          </>
        )}
      </Stack>
    </Card>
  );
}
