"use client";

import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { Selector } from "@astryxdesign/core/Selector";
import { Stack } from "@astryxdesign/core/Stack";
import { Switch } from "@astryxdesign/core/Switch";
import { Text } from "@astryxdesign/core/Text";
import { Token } from "@astryxdesign/core/Token";
import { Toolbar } from "@astryxdesign/core/Toolbar";
import { Download } from "lucide-react";
import { useTranslations } from "next-intl";
import { ChatLog } from "@/components/ChatLog";
import { Page } from "@/components/Page";
import { SearchField } from "@/components/SearchField";
import { useQueryPatch } from "@/components/useQueryPatch";
import type { ChatLine } from "@/lib/chat/present";

const RANGES = ["1h", "24h", "7d", "30d"] as const;

export function ChatClient({
  lines,
  hasMore,
  filters,
  exportHref,
}: {
  lines: ChatLine[];
  hasMore: boolean;
  filters: {
    user: string;
    name: string;
    room: string;
    channel: string;
    q: string;
    flagged: boolean;
    range: string;
  };
  exportHref: string | null;
}) {
  const t = useTranslations("chat");
  const tc = useTranslations("common");
  const patch = useQueryPatch();
  const oldest = lines.at(-1)?.id;

  return (
    <Page
      title={t("title")}
      description={t("description")}
      actions={
        exportHref ? (
          <Button label={tc("export")} icon={<Download size={16} />} href={exportHref} />
        ) : undefined
      }
    >
      <Toolbar
        label={t("filters")}
        startContent={
          <>
            <SearchField
              label={t("searchText")}
              placeholder={t("searchText")}
              value={filters.q}
              onCommit={(q) => patch({ q })}
            />
            <SearchField
              label={t("searchPlayer")}
              placeholder={t("searchPlayer")}
              value={filters.name}
              onCommit={(name) => patch({ name })}
              width={200}
            />
            <Selector
              label={t("columns.channel")}
              isLabelHidden
              size="sm"
              variant="secondary"
              hasClear
              placeholder={t("anyChannel")}
              value={filters.channel || null}
              options={(["city", "party", "guild", "direct", "system"] as const).map((c) => ({
                value: c,
                label: tc(`channels.${c}`),
              }))}
              onChange={(v) => patch({ channel: v })}
            />
            <Selector
              label={tc("ranges.label")}
              isLabelHidden
              size="sm"
              variant="secondary"
              value={filters.range || "all"}
              options={[
                ...RANGES.map((r) => ({ value: r, label: tc(`ranges.${r}`) })),
                { value: "all", label: tc("ranges.all") },
              ]}
              onChange={(v) => patch({ range: v === "all" ? null : v })}
            />
          </>
        }
        endContent={
          <Switch
            label={t("flaggedOnly")}
            value={filters.flagged}
            onChange={(on) => patch({ flagged: on })}
            size="sm"
          />
        }
      />

      {(filters.user || filters.room) && (
        <Stack direction="horizontal" gap={2} align="center">
          <Text type="supporting">{t("scopedTo")}</Text>
          {filters.user && (
            <Token
              label={t("userScope", { id: filters.user })}
              onRemove={() => patch({ user: null })}
            />
          )}
          {filters.room && (
            <Token
              label={t("roomScope", { id: filters.room })}
              onRemove={() => patch({ room: null })}
            />
          )}
        </Stack>
      )}

      <Card padding={0}>
        <ChatLog lines={lines} emptyTitle={t("noMatches")} />
      </Card>

      {hasMore && oldest && (
        <Stack align="center">
          <Button label={tc("loadMore")} onClick={() => patch({ before: oldest })} />
        </Stack>
      )}
    </Page>
  );
}
