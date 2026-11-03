"use client";

import { AlertDialog } from "@astryxdesign/core/AlertDialog";
import { Badge } from "@astryxdesign/core/Badge";
import { Button } from "@astryxdesign/core/Button";
import { Card } from "@astryxdesign/core/Card";
import { EmptyState } from "@astryxdesign/core/EmptyState";
import { Grid } from "@astryxdesign/core/Grid";
import { Heading } from "@astryxdesign/core/Heading";
import { IconButton } from "@astryxdesign/core/IconButton";
import { Selector } from "@astryxdesign/core/Selector";
import { Stack } from "@astryxdesign/core/Stack";
import { Switch } from "@astryxdesign/core/Switch";
import { pixel, proportional, Table } from "@astryxdesign/core/Table";
import { Text } from "@astryxdesign/core/Text";
import { TextArea } from "@astryxdesign/core/TextArea";
import { Token } from "@astryxdesign/core/Token";
import { Toolbar } from "@astryxdesign/core/Toolbar";
import { Pencil, Plus, Trash2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useMemo, useState } from "react";
import { Page } from "@/components/Page";
import { RevisionList, type RevisionRow } from "@/components/RevisionList";
import { SearchField } from "@/components/SearchField";
import { useAction } from "@/components/useAction";
import type { BlacklistTerm } from "@/lib/db/schema";
import { deleteTermAction, testAction, updateTermAction } from "./actions";
import { TermDialog } from "./TermDialog";

export const ACTION_BADGE = { flag: "warning", mask: "orange", block: "error" } as const;
const VERDICT_BADGE = {
  allowed: "success",
  flagged: "warning",
  masked: "orange",
  blocked: "error",
} as const;

type Row = BlacklistTerm & Record<string, unknown>;

export function ChatFilterClient({
  terms,
  version,
  revisions,
  canWrite,
  canRestore,
}: {
  terms: BlacklistTerm[];
  version: number;
  revisions: RevisionRow[];
  canWrite: boolean;
  canRestore: boolean;
}) {
  const t = useTranslations("filter");
  const tc = useTranslations("common");
  const { run } = useAction();
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState<string | null>(null);
  const [editing, setEditing] = useState<BlacklistTerm | "new" | null>(null);
  const [deleting, setDeleting] = useState<BlacklistTerm | null>(null);
  const [sample, setSample] = useState("");
  const [sampleChannel, setSampleChannel] = useState("city");
  const [result, setResult] = useState<{
    verdict: keyof typeof VERDICT_BADGE;
    masked: string;
    termIds: number[];
  } | null>(null);

  const categories = useMemo(() => [...new Set(terms.map((x) => x.category))].sort(), [terms]);
  const shown = terms.filter(
    (x) =>
      (!category || x.category === category) &&
      (!search ||
        x.pattern.toLowerCase().includes(search.toLowerCase()) ||
        x.note?.toLowerCase().includes(search.toLowerCase())),
  );

  const toInput = (x: BlacklistTerm) => ({
    pattern: x.pattern,
    matchType: x.matchType,
    action: x.action,
    category: x.category,
    note: x.note,
    enabled: x.enabled,
    channels: x.channels,
  });

  return (
    <Page
      title={t("title")}
      description={t("description", { version, count: terms.length })}
      actions={
        canWrite ? (
          <Button
            label={t("add")}
            variant="primary"
            icon={<Plus size={16} />}
            onClick={() => setEditing("new")}
          />
        ) : undefined
      }
    >
      <Grid columns={{ minWidth: 360, repeat: "fit" }} gap={4}>
        <Card padding={4}>
          <Stack gap={3}>
            <Heading level={2}>{t("tester")}</Heading>
            <Text type="supporting">{t("testerHint")}</Text>
            <TextArea
              label={t("sample")}
              isLabelHidden
              value={sample}
              onChange={setSample}
              rows={3}
              placeholder={t("samplePlaceholder")}
            />
            <Stack direction="horizontal" gap={2} align="center" justify="between">
              <Selector
                label={t("channel")}
                isLabelHidden
                size="sm"
                variant="secondary"
                value={sampleChannel}
                options={(["city", "party", "guild", "direct"] as const).map((c) => ({
                  value: c,
                  label: tc(`channels.${c}`),
                }))}
                onChange={setSampleChannel}
              />
              <Button
                label={t("test")}
                isDisabled={!sample.trim()}
                clickAction={async () => {
                  const r = await run(() => testAction(sample, sampleChannel));
                  if (r !== false) setResult(r as typeof result);
                }}
              />
            </Stack>
            {result && (
              <Stack gap={2}>
                <Stack direction="horizontal" gap={2} align="center">
                  <Badge
                    variant={VERDICT_BADGE[result.verdict]}
                    label={t(`verdicts.${result.verdict}`)}
                  />
                  {result.termIds.length > 0 && (
                    <Text type="supporting">
                      {t("matchedTerms", { count: result.termIds.length })}
                    </Text>
                  )}
                </Stack>
                {result.verdict !== "allowed" && <Text type="code">{result.masked}</Text>}
              </Stack>
            )}
          </Stack>
        </Card>
        <Card padding={4}>
          <Stack gap={3}>
            <Heading level={2}>{t("howItWorks")}</Heading>
            <Stack gap={1}>
              {(["flag", "mask", "block"] as const).map((a) => (
                <Stack key={a} direction="horizontal" gap={2} align="start">
                  <Badge variant={ACTION_BADGE[a]} label={t(`actions.${a}`)} />
                  <Text type="supporting">{t(`actionHelp.${a}`)}</Text>
                </Stack>
              ))}
            </Stack>
            <Text type="supporting">{t("enforcement")}</Text>
          </Stack>
        </Card>
      </Grid>

      <Toolbar
        label={t("filters")}
        startContent={
          <>
            <SearchField
              label={tc("search")}
              placeholder={t("searchTerms")}
              value={search}
              onCommit={setSearch}
            />
            <Selector
              label={t("category")}
              isLabelHidden
              size="sm"
              variant="secondary"
              hasClear
              placeholder={t("anyCategory")}
              value={category}
              options={categories.map((c) => ({ value: c, label: c }))}
              onChange={(v) => setCategory(v || null)}
            />
          </>
        }
      />

      <Card padding={0}>
        {shown.length === 0 ? (
          <EmptyState
            title={terms.length ? t("noMatches") : t("empty")}
            description={terms.length ? undefined : t("emptyHint")}
          />
        ) : (
          <Table<Row>
            data={shown as Row[]}
            idKey="id"
            density="compact"
            hasHover
            columns={[
              {
                key: "pattern",
                header: t("columns.pattern"),
                width: proportional(2),
                renderCell: (x) => (
                  <Stack gap={0.5}>
                    <Text type="code" hasStrikethrough={!x.enabled}>
                      {x.pattern}
                    </Text>
                    {x.note && <Text type="supporting">{x.note}</Text>}
                  </Stack>
                ),
              },
              {
                key: "matchType",
                header: t("columns.kind"),
                width: pixel(110),
                renderCell: (x) => t(`kinds.${x.matchType}`),
              },
              {
                key: "action",
                header: t("columns.action"),
                width: pixel(100),
                renderCell: (x) => (
                  <Badge variant={ACTION_BADGE[x.action]} label={t(`actions.${x.action}`)} />
                ),
              },
              {
                key: "channels",
                header: t("columns.channels"),
                width: proportional(1),
                renderCell: (x) =>
                  x.channels ? (
                    <Stack direction="horizontal" gap={1} wrap="wrap">
                      {x.channels.map((c) => (
                        <Token key={c} size="sm" label={tc(`channels.${c as "city"}`)} />
                      ))}
                    </Stack>
                  ) : (
                    <Text type="supporting">{t("allChannels")}</Text>
                  ),
              },
              {
                key: "category",
                header: t("columns.category"),
                width: pixel(120),
                renderCell: (x) => <Token size="sm" label={x.category} />,
              },
              {
                key: "enabled",
                header: t("columns.enabled"),
                width: pixel(90),
                renderCell: (x) => (
                  <Switch
                    label={t("columns.enabled")}
                    isLabelHidden
                    size="sm"
                    value={x.enabled}
                    isDisabled={!canWrite}
                    changeAction={async (on) =>
                      void (await run(() => updateTermAction(x.id, { ...toInput(x), enabled: on })))
                    }
                  />
                ),
              },
              {
                key: "edit",
                header: "",
                width: pixel(90),
                align: "end",
                renderCell: (x) =>
                  canWrite ? (
                    <Stack direction="horizontal" gap={0.5} justify="end">
                      <IconButton
                        label={tc("edit")}
                        icon={<Pencil size={14} />}
                        variant="ghost"
                        size="sm"
                        onClick={() => setEditing(x)}
                      />
                      <IconButton
                        label={tc("delete")}
                        icon={<Trash2 size={14} />}
                        variant="ghost"
                        size="sm"
                        onClick={() => setDeleting(x)}
                      />
                    </Stack>
                  ) : null,
              },
            ]}
          />
        )}
      </Card>

      <Card padding={0}>
        <Stack paddingInline={4} paddingBlock={3} gap={1}>
          <Heading level={2}>{t("history")}</Heading>
          <Text type="supporting">{t("historyHint")}</Text>
        </Stack>
        <RevisionList revisions={revisions} canRestore={canRestore} />
      </Card>

      <TermDialog term={editing} categories={categories} onClose={() => setEditing(null)} />
      <AlertDialog
        isOpen={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
        title={t("deleteTitle")}
        description={t("deleteBody", { pattern: deleting?.pattern ?? "" })}
        actionLabel={tc("delete")}
        onAction={async () => {
          if (!deleting) return;
          if ((await run(() => deleteTermAction(deleting.id), t("deleted"))) !== false)
            setDeleting(null);
        }}
      />
    </Page>
  );
}
