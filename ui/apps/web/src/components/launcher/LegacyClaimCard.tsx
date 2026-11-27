"use client";

import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Heading } from "@astryxdesign/core/Heading";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import type { LegacyClaimAnswer } from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { useTranslations } from "next-intl";
import { useState } from "react";

/** A Ramsgate Reborn claim code, entered before the first play so the old save comes with it. */
export function LegacyClaimCard() {
  const t = useTranslations("launcher.legacyClaim");
  const { snapshot, api } = useBridge();
  const [code, setCode] = useState("");
  const [answer, setAnswer] = useState<LegacyClaimAnswer | null>(null);
  const [busy, setBusy] = useState(false);
  if (!api || snapshot?.account.hasKey !== true || snapshot.server?.name === undefined) return null;

  const redeem = async () => {
    setBusy(true);
    try {
      setAnswer(await api.redeemLegacyClaim(code));
    } catch {
      setAnswer("unavailable");
    } finally {
      setBusy(false);
    }
  };
  const claimed = answer === "claimed";

  return (
    <Card padding={5}>
      <Stack gap={3}>
        <Heading level={2}>{t("title")}</Heading>
        <Text type="supporting">{t("hint")}</Text>
        {!claimed && (
          <Stack direction="horizontal" gap={2}>
            <TextInput
              label={t("code")}
              value={code}
              onChange={setCode}
              onEnter={() => code.trim() && !busy && void redeem()}
              status={answer ? { type: "error", message: t(`answers.${answer}`) } : undefined}
            />
            <Button label={t("redeem")} isDisabled={!code.trim() || busy} clickAction={redeem} />
          </Stack>
        )}
        {claimed && <Text weight="semibold">{t("answers.claimed")}</Text>}
      </Stack>
    </Card>
  );
}
