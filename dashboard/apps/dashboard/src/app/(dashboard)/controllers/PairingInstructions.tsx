"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { CodeBlock } from "@astryxdesign/core/CodeBlock";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { useTranslations } from "next-intl";

/** Shown once after a code is made: the code is not stored readably, so it cannot be shown again. */
export function PairingInstructions({
  code,
  expiresAt,
  dashboardUrl,
  insecure,
}: {
  code: string;
  expiresAt: number | null;
  dashboardUrl: string;
  insecure: boolean;
}) {
  const t = useTranslations("fleet.pairing");
  const env = [
    `DAINTY_AGENT_DASHBOARD_URL=${dashboardUrl}`,
    `DAINTY_AGENT_PAIRING_CODE=${code}`,
    "# from that controller's secrets.env:",
    "DAINTY_AGENT_ADMIN_API_KEY=<REFORGED_ADMIN_API_KEY>",
    "DAINTY_AGENT_OPERATOR_SECRET=<RUSTIC_ISLES_MANAGER_OPERATOR_SECRET>",
  ].join("\n");
  return (
    <Stack gap={3}>
      {insecure && <Banner status="warning" title={t("insecure")} collapsible={false} />}
      <Stack gap={1}>
        <Text type="label">{t("code")}</Text>
        <Text type="display-3" hasTabularNumbers>
          {code}
        </Text>
        {expiresAt && (
          <Text type="supporting">
            {t("expires")}{" "}
            <Timestamp value={new Date(expiresAt).toISOString()} format="relative" isLive />
          </Text>
        )}
      </Stack>
      <Text>{t("step1")}</Text>
      <CodeBlock code={env} language="bash" title="agent.env" />
      <Text>{t("step2")}</Text>
      <CodeBlock code="bun --env-file=agent.env apps/agent/src/main.ts" language="bash" />
      <Text type="supporting">{t("after")}</Text>
    </Stack>
  );
}
