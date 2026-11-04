"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { TextArea } from "@astryxdesign/core/TextArea";
import { HUES } from "@dainty/theme";
import { LockKeyhole } from "lucide-react";
import { useTranslations } from "next-intl";
import { useState } from "react";
import { readDirectAction } from "@/app/(dashboard)/players/actions";
import type { ChatLine } from "@/lib/chat/present";
import { ChatLog } from "./ChatLog";
import { useAction } from "./useAction";

/**
 * A player's direct messages behind a reason. Nothing is fetched until staff ask, each ask is one audit
 * event, and the lines live only in this component, so leaving the page closes them again.
 */
export function DirectMessages({
  controllerId,
  userId,
  name,
  reportId,
}: {
  controllerId: string;
  userId: string;
  name: string;
  reportId?: number;
}) {
  const t = useTranslations("direct");
  const { run } = useAction();
  const [reason, setReason] = useState(reportId ? t("reportReason", { id: reportId }) : "");
  const [busy, setBusy] = useState(false);
  const [read, setRead] = useState<{ lines: ChatLine[]; hasMore: boolean } | null>(null);

  if (read)
    return (
      <Stack gap={0}>
        <Stack padding={3}>
          <Banner
            status="info"
            title={t("recorded")}
            description={read.hasMore ? t("truncated") : undefined}
            collapsible={false}
            endContent={<Button label={t("close")} size="sm" onClick={() => setRead(null)} />}
          />
        </Stack>
        <ChatLog lines={read.lines} emptyTitle={t("none", { name })} showRoom={false} />
      </Stack>
    );

  return (
    <Stack gap={3} padding={4} maxWidth={560}>
      <Stack direction="horizontal" gap={2} align="center">
        <LockKeyhole size={18} className={HUES.orange.text} />
        <Text weight="semibold">{t("title", { name })}</Text>
      </Stack>
      <Text type="supporting">{t("explain")}</Text>
      <TextArea
        label={t("reason")}
        value={reason}
        onChange={setReason}
        rows={2}
        maxLength={500}
        isRequired
      />
      <Stack direction="horizontal" justify="end">
        <Button
          label={t("read")}
          variant="primary"
          isLoading={busy}
          isDisabled={reason.trim().length < 3}
          onClick={async () => {
            setBusy(true);
            const result = await run(() =>
              readDirectAction({ controllerId, userId, name, reason, reportId }),
            );
            setBusy(false);
            if (result !== false) setRead(result);
          }}
        />
      </Stack>
    </Stack>
  );
}
