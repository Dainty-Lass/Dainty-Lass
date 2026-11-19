"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Stack } from "@astryxdesign/core/Stack";
import { useBridge } from "@dainty/bridge/react";
import { useTranslations } from "next-intl";

/** The last game crash: asks to send its report, then says how sending went. */
export function CrashReportBanner() {
  const t = useTranslations("launcher.crashReports");
  const ts = useTranslations("launcher.shell");
  const { snapshot, api } = useBridge();
  const crash = snapshot?.crash;
  if (!crash || !api) return null;
  const server = snapshot.settings.streamerMode ? ts("hidden") : (snapshot.server?.name ?? "");
  const dismiss = () => void api.crashReportDismiss();
  const send = (label: string, variant: "primary" | "secondary") => (
    <Button
      label={label}
      size="sm"
      variant={variant}
      clickAction={async () => void (await api.crashReportSend())}
    />
  );

  switch (crash.state) {
    case "ask":
      return (
        <Banner
          status="warning"
          container="section"
          title={t("ask", { server })}
          collapsible={false}
          endContent={
            <Stack direction="horizontal" gap={2}>
              {send(t("send"), "primary")}
              <Button label={t("dontSend")} size="sm" onClick={dismiss} />
            </Stack>
          }
        />
      );
    case "sending":
      return <Banner status="info" container="section" title={t("sending")} collapsible={false} />;
    case "sent":
      return (
        <Banner
          status="success"
          container="section"
          title={t("sent")}
          collapsible={false}
          isDismissable
          onDismiss={dismiss}
        />
      );
    default:
      return (
        <Banner
          status="error"
          container="section"
          title={t(crash.state)}
          collapsible={false}
          isDismissable
          onDismiss={dismiss}
          endContent={send(t("retry"), "secondary")}
        />
      );
  }
}
