"use client";

import { Banner } from "@astryxdesign/core/Banner";
import { Button } from "@astryxdesign/core/Button";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { IconButton } from "@astryxdesign/core/IconButton";
import { List, ListItem } from "@astryxdesign/core/List";
import { NumberInput } from "@astryxdesign/core/NumberInput";
import { SegmentedControl, SegmentedControlItem } from "@astryxdesign/core/SegmentedControl";
import { Selector } from "@astryxdesign/core/Selector";
import { Spinner } from "@astryxdesign/core/Spinner";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { Typeahead } from "@astryxdesign/core/Typeahead";
import { Trash2 } from "lucide-react";
import { useTranslations } from "next-intl";
import { useEffect, useMemo, useState } from "react";
import { ReasonField } from "@/components/ReasonField";
import { useAction } from "@/components/useAction";
import type { BackendAccount, GrantCatalog, GrantReport } from "@/lib/game/backend";
import { grantAction, grantCatalogAction } from "../actions";

type Kind = "item" | "currency" | "level";
type Line =
  | { kind: "item"; id: string; quantity: number }
  | { kind: "currency"; id: string; quantity: number }
  | { kind: "level"; id: string; mode: "xp" | "rank"; amount: number };

// The backend's limits (rustic-isles-launcher-api admin/grants.rs)
const MAX = { item: 1000, currency: 10_000_000, xp: 10_000_000 };
// Currencies staff know by name; the rest show their catalog id
const NAMED: Record<string, "rams" | "platinum"> = {
  CURRENCY_NOTES: "rams",
  CURRENCY_PLATINUM_UNIV: "platinum",
};

type Option = { id: string; label: string };

/** Items, currencies and levels for one account, through the backend's own grant path. */
export function GrantDialog({
  isOpen,
  controllerId,
  account,
  online,
  reasons,
  onClose,
}: {
  isOpen: boolean;
  controllerId: string;
  account: BackendAccount;
  online: boolean;
  reasons: string[];
  onClose: () => void;
}) {
  const t = useTranslations("grants");
  const tp = useTranslations("players");
  const te = useTranslations("errors");
  const { run } = useAction();
  const [catalog, setCatalog] = useState<GrantCatalog | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [kind, setKind] = useState<Kind>("item");
  const [item, setItem] = useState<Option | null>(null);
  const [currency, setCurrency] = useState<string | undefined>();
  const [track, setTrack] = useState<string | undefined>();
  const [mode, setMode] = useState<"xp" | "rank">("rank");
  const [amount, setAmount] = useState(1);
  const [lines, setLines] = useState<Line[]>([]);
  const [reason, setReason] = useState("");
  const [step, setStep] = useState<"pick" | "confirm" | "done">("pick");
  const [grantId, setGrantId] = useState(() => crypto.randomUUID());
  const [report, setReport] = useState<GrantReport | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!isOpen || catalog) return;
    let live = true;
    void grantCatalogAction(controllerId).then((r) => {
      if (!live) return;
      if (r.ok && r.value) setCatalog(r.value);
      else if (!r.ok) setLoadError(te(r.code, { detail: r.detail ?? "" }));
    });
    return () => {
      live = false;
    };
  }, [isOpen, catalog, controllerId, te]);

  // A changed grant is a new grant: the id only replays the exact request it was made for.
  // biome-ignore lint/correctness/useExhaustiveDependencies: lines is the trigger
  useEffect(() => setGrantId(crypto.randomUUID()), [lines]);

  const itemSource = useMemo(() => {
    const all: Option[] = (catalog?.items ?? []).map((i) => ({
      id: i.catalogId,
      label: i.catalogId,
    }));
    return {
      search: (query: string) => {
        const q = query.trim().toLowerCase();
        return all.filter((i) => i.id.toLowerCase().includes(q)).slice(0, 50);
      },
      bootstrap: () => all.slice(0, 50),
    };
  }, [catalog]);

  const currencyLabel = (id: string) =>
    NAMED[id] ? t("currencyNamed", { name: t(`currencyNames.${NAMED[id]}`), id }) : id;
  const maxRank = (id: string) =>
    catalog?.progression.find((p) => p.progressionId === id)?.maxRank ?? null;

  const limit =
    kind === "item"
      ? MAX.item
      : kind === "currency"
        ? MAX.currency
        : mode === "xp"
          ? MAX.xp
          : (maxRank(track ?? "") ?? 1);
  const picked = kind === "item" ? item?.id : kind === "currency" ? currency : track;

  function reset() {
    setLines([]);
    setReason("");
    setItem(null);
    setAmount(1);
    setStep("pick");
    setReport(null);
  }

  function close() {
    reset();
    onClose();
  }

  function add() {
    if (!picked) return;
    const n = Math.max(1, Math.min(limit, Math.round(amount)));
    setLines((prev) => {
      if (kind === "level")
        return [
          ...prev.filter((l) => !(l.kind === "level" && l.id === picked)),
          { kind, id: picked, mode, amount: n },
        ];
      const same = prev.find((l) => l.kind === kind && l.id === picked);
      if (same)
        return prev.map((l) =>
          l === same && l.kind !== "level"
            ? { ...l, quantity: Math.min(limit, l.quantity + n) }
            : l,
        );
      return [...prev, { kind, id: picked, quantity: n }];
    });
    setItem(null);
    setAmount(1);
  }

  function describe(line: Line) {
    if (line.kind === "item") return t("line.item", { id: line.id, quantity: line.quantity });
    if (line.kind === "currency")
      return t("line.currency", { name: currencyLabel(line.id), quantity: line.quantity });
    return line.mode === "xp"
      ? t("line.xp", { id: line.id, amount: line.amount })
      : t("line.rank", { id: line.id, rank: line.amount });
  }

  async function submit() {
    setBusy(true);
    const stacks = (k: "item" | "currency") =>
      lines.flatMap((l) => (l.kind === k ? [{ catalogId: l.id, quantity: l.quantity }] : []));
    const progression = lines.flatMap((l) =>
      l.kind === "level"
        ? [
            l.mode === "xp"
              ? { progressionId: l.id, xp: l.amount }
              : { progressionId: l.id, rank: l.amount },
          ]
        : [],
    );
    const done = await run(() =>
      grantAction(
        controllerId,
        account.userId,
        grantId,
        {
          items: stacks("item").length ? stacks("item") : undefined,
          currencies: stacks("currency").length ? stacks("currency") : undefined,
          progression: progression.length ? progression : undefined,
        },
        reason,
      ),
    );
    setBusy(false);
    if (done !== false) {
      setReport(done);
      setStep("done");
    }
  }

  const footer = (buttons: React.ReactNode) => (
    <Stack direction="horizontal" gap={2} justify="end" paddingInline={4} paddingBlockEnd={4}>
      {buttons}
    </Stack>
  );

  return (
    <Dialog isOpen={isOpen} onOpenChange={(open) => !open && close()} purpose="form" width={560}>
      <Stack gap={4}>
        <DialogHeader
          title={t("title", { name: account.displayName })}
          subtitle={t("explain")}
          onOpenChange={() => close()}
        />
        {step === "pick" && (
          <>
            <Stack gap={4} paddingInline={4}>
              {loadError && <Banner status="error" title={loadError} collapsible={false} />}
              {!catalog && !loadError && <Spinner label={t("loading")} />}
              {catalog && (
                <>
                  <SegmentedControl
                    label={t("kind")}
                    value={kind}
                    onChange={(v) => {
                      setKind(v as Kind);
                      setAmount(1);
                    }}
                    layout="fill"
                  >
                    {(["item", "currency", "level"] as const).map((k) => (
                      <SegmentedControlItem key={k} value={k} label={t(`kinds.${k}`)} />
                    ))}
                  </SegmentedControl>
                  {kind === "item" && (
                    <Typeahead
                      label={t("item")}
                      placeholder={t("itemPlaceholder")}
                      searchSource={itemSource}
                      value={item}
                      onChange={setItem}
                      hasEntriesOnFocus
                      maxMenuItems={20}
                      debounceMs={0}
                      emptySearchText={t("noItems")}
                      width="100%"
                    />
                  )}
                  {kind === "currency" && (
                    <Selector
                      label={t("currency")}
                      placeholder={t("currencyPlaceholder")}
                      value={currency}
                      onChange={(v) => setCurrency(v as string)}
                      options={catalog.currencies.map((id) => ({
                        value: id,
                        label: currencyLabel(id),
                      }))}
                      width="100%"
                    />
                  )}
                  {kind === "level" && (
                    <>
                      <Selector
                        label={t("track")}
                        placeholder={t("trackPlaceholder")}
                        value={track}
                        onChange={(v) => setTrack(v as string)}
                        options={catalog.progression.map((p) => ({
                          value: p.progressionId,
                          label:
                            p.maxRank === null
                              ? p.progressionId
                              : t("trackOption", { id: p.progressionId, max: p.maxRank }),
                        }))}
                        width="100%"
                      />
                      <SegmentedControl
                        label={t("levelMode")}
                        value={mode}
                        onChange={(v) => {
                          setMode(v as "xp" | "rank");
                          setAmount(1);
                        }}
                        layout="fill"
                      >
                        <SegmentedControlItem value="rank" label={t("modes.rank")} />
                        <SegmentedControlItem value="xp" label={t("modes.xp")} />
                      </SegmentedControl>
                    </>
                  )}
                  <Stack direction="horizontal" gap={2} align="end">
                    <NumberInput
                      label={
                        kind === "level"
                          ? t(mode === "xp" ? "xpAmount" : "rankAmount")
                          : t("quantity")
                      }
                      description={t("upTo", { max: limit })}
                      value={amount}
                      onChange={(v) => setAmount(Number.isFinite(v) ? v : 1)}
                      width="100%"
                    />
                    <Button label={t("add")} onClick={add} isDisabled={!picked} />
                  </Stack>
                  {lines.length > 0 ? (
                    <List>
                      {lines.map((line) => (
                        <ListItem
                          key={`${line.kind}:${line.id}`}
                          label={describe(line)}
                          endContent={
                            <IconButton
                              label={t("remove")}
                              icon={<Trash2 size={14} />}
                              variant="ghost"
                              size="sm"
                              onClick={() => setLines((prev) => prev.filter((l) => l !== line))}
                            />
                          }
                        />
                      ))}
                    </List>
                  ) : (
                    <Text type="supporting">{t("empty")}</Text>
                  )}
                  <ReasonField
                    label={t("reason")}
                    presets={reasons}
                    value={reason}
                    onChange={setReason}
                  />
                </>
              )}
            </Stack>
            {footer(
              <>
                <Button label={tp("cancel")} variant="ghost" onClick={close} />
                <Button
                  label={t("review")}
                  variant="primary"
                  isDisabled={lines.length === 0 || reason.trim().length < 3}
                  onClick={() => setStep("confirm")}
                />
              </>,
            )}
          </>
        )}
        {step === "confirm" && (
          <>
            <Stack gap={4} paddingInline={4}>
              <Banner
                status="warning"
                title={t("confirmTitle", { name: account.displayName })}
                description={online ? t("whenOnline") : t("whenOffline")}
                collapsible={false}
              />
              <List>
                {lines.map((line) => (
                  <ListItem key={`${line.kind}:${line.id}`} label={describe(line)} />
                ))}
              </List>
              <Text type="supporting">{t("reasonShown", { reason: reason.trim() })}</Text>
            </Stack>
            {footer(
              <>
                <Button label={t("back")} variant="ghost" onClick={() => setStep("pick")} />
                <Button label={t("confirm")} variant="primary" isLoading={busy} onClick={submit} />
              </>,
            )}
          </>
        )}
        {step === "done" && report && (
          <>
            <Stack gap={4} paddingInline={4}>
              <Banner
                status={report.status === "done" ? "success" : "warning"}
                title={report.status === "done" ? t("doneTitle") : t("partialTitle")}
                description={online ? t("whenOnline") : t("whenOffline")}
                collapsible={false}
              />
              <List>
                {report.items && (
                  <ListItem
                    label={t("result.items", { count: report.items.stacks.length })}
                    description={report.items.error}
                    endContent={
                      <Text type="supporting">{t(`result.status.${report.items.status}`)}</Text>
                    }
                  />
                )}
                {report.progression.map((row) => (
                  <ListItem
                    key={row.progressionId}
                    label={t("result.track", {
                      id: row.progressionId,
                      granted: row.granted,
                      rank: row.confirmedRank ?? 0,
                    })}
                    description={row.error}
                    endContent={<Text type="supporting">{t(`result.status.${row.status}`)}</Text>}
                  />
                ))}
              </List>
            </Stack>
            {footer(<Button label={t("close")} variant="primary" onClick={close} />)}
          </>
        )}
      </Stack>
    </Dialog>
  );
}
