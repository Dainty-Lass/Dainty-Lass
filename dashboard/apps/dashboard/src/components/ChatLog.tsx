"use client";

import { Badge } from "@astryxdesign/core/Badge";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Link } from "@astryxdesign/core/Link";
import { Stack } from "@astryxdesign/core/Stack";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { Timestamp } from "@astryxdesign/core/Timestamp";
import { Token } from "@astryxdesign/core/Token";
import { useTranslations } from "next-intl";
import type { ChatLine } from "@/lib/chat/present";
import { ControllerLabel, useControllers } from "./ControllerContext";

const CHANNEL_COLOR = {
  city: "blue",
  party: "green",
  guild: "purple",
  direct: "orange",
  system: "gray",
} as const;
const VERDICT = { flagged: "warning", masked: "orange", blocked: "error" } as const;

type Row = ChatLine & Record<string, unknown>;

/** A chat transcript as rows: dense, scannable, and with the filter's hits marked inside the text. */
export function ChatLog({
  lines,
  highlightId,
  emptyTitle,
  showRoom = true,
}: {
  lines: ChatLine[];
  /** The line a report or link pointed at. */
  highlightId?: number;
  emptyTitle?: string;
  showRoom?: boolean;
}) {
  const t = useTranslations("chat");
  const tc = useTranslations("common");
  const tf = useTranslations("fleet");
  const { fleetWide } = useControllers();
  if (lines.length === 0) return <EmptyState isCompact title={emptyTitle ?? t("empty")} />;
  return (
    <Table<Row>
      data={lines as Row[]}
      idKey="id"
      density="compact"
      verticalAlign="top"
      hasHover
      columns={[
        {
          key: "at",
          header: t("columns.time"),
          width: pixel(110),
          renderCell: (m) => (
            <Timestamp value={new Date(m.at).toISOString()} format="time" type="code" />
          ),
        },
        {
          key: "channel",
          header: t("columns.channel"),
          width: pixel(100),
          renderCell: (m) => (
            <Token
              size="sm"
              label={tc(`channels.${m.channel}`)}
              color={CHANNEL_COLOR[m.channel]}
              href={showRoom && m.roomId ? `/chat?room=${encodeURIComponent(m.roomId)}` : undefined}
            />
          ),
        },
        ...(fleetWide
          ? [
              {
                key: "controllerId",
                header: tf("columns.controller"),
                width: pixel(140),
                renderCell: (m: Row) => <ControllerLabel id={m.controllerId} />,
              },
            ]
          : []),
        {
          key: "sender",
          header: t("columns.from"),
          width: pixel(170),
          renderCell: (m) => (
            <Stack gap={0}>
              <Link href={`/players/${encodeURIComponent(m.senderName)}?c=${m.controllerId}`}>
                {m.senderName}
              </Link>
              {m.recipientName && (
                <Text type="supporting" size="xsm">
                  {t("to", { name: m.recipientName })}
                </Text>
              )}
            </Stack>
          ),
        },
        {
          key: "body",
          header: t("columns.message"),
          width: proportional(4),
          renderCell: (m) => (
            <Stack direction="horizontal" gap={2} align="start" justify="between">
              <Text weight={m.id === highlightId ? "semibold" : undefined}>
                {m.segments.map((s, i) =>
                  s.hit ? (
                    // biome-ignore lint/suspicious/noArrayIndexKey: segments are positional and never reorder.
                    <mark key={i} className="dainty-chat-hit">
                      {s.text}
                    </mark>
                  ) : (
                    // biome-ignore lint/suspicious/noArrayIndexKey: as above.
                    <span key={i}>{s.text}</span>
                  ),
                )}
              </Text>
              {m.verdict !== "allowed" && (
                <Badge variant={VERDICT[m.verdict]} label={t(`verdicts.${m.verdict}`)} />
              )}
            </Stack>
          ),
        },
      ]}
    />
  );
}
