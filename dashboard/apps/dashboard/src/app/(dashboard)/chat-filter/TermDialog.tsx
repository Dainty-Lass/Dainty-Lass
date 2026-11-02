"use client";

import { Button } from "@astryxdesign/core/Button";
import { CheckboxList, CheckboxListItem } from "@astryxdesign/core/CheckboxList";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { SegmentedControl, SegmentedControlItem } from "@astryxdesign/core/SegmentedControl";
import { Stack } from "@astryxdesign/core/Stack";
import { Switch } from "@astryxdesign/core/Switch";
import { TextInput } from "@astryxdesign/core/TextInput";
import { useTranslations } from "next-intl";
import { useEffect, useState } from "react";
import { useAction } from "@/components/useAction";
import type { BlacklistTerm } from "@/lib/db/schema";
import { createTermAction, updateTermAction } from "./actions";

const CHANNELS = ["city", "party", "guild", "direct"] as const;

function regexError(pattern: string): boolean {
  try {
    new RegExp(pattern, "u");
    return false;
  } catch {
    return true;
  }
}

export function TermDialog({
  term,
  categories,
  onClose,
}: {
  term: BlacklistTerm | "new" | null;
  categories: string[];
  onClose: () => void;
}) {
  const t = useTranslations("filter");
  const tc = useTranslations("common");
  const { run } = useAction();
  const editing = term && term !== "new" ? term : null;
  const [pattern, setPattern] = useState("");
  const [matchType, setMatchType] = useState<BlacklistTerm["matchType"]>("word");
  const [action, setAction] = useState<BlacklistTerm["action"]>("mask");
  const [category, setCategory] = useState("general");
  const [note, setNote] = useState("");
  const [enabled, setEnabled] = useState(true);
  const [channels, setChannels] = useState<string[]>([...CHANNELS]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setPattern(editing?.pattern ?? "");
    setMatchType(editing?.matchType ?? "word");
    setAction(editing?.action ?? "mask");
    setCategory(editing?.category ?? "general");
    setNote(editing?.note ?? "");
    setEnabled(editing?.enabled ?? true);
    setChannels(editing?.channels ?? [...CHANNELS]);
  }, [editing]);

  const badRegex = matchType === "regex" && pattern.length > 0 && regexError(pattern);

  async function save() {
    setBusy(true);
    const input = {
      pattern,
      matchType,
      action,
      category,
      note: note || null,
      enabled,
      channels: channels.length === CHANNELS.length ? null : channels,
    };
    const ok = await run(
      () => (editing ? updateTermAction(editing.id, input) : createTermAction(input)),
      editing ? t("saved") : t("added"),
    );
    setBusy(false);
    if (ok !== false) onClose();
  }

  return (
    <Dialog
      isOpen={term !== null}
      onOpenChange={(open) => !open && onClose()}
      purpose="form"
      width={520}
    >
      {term && (
        <Stack gap={4}>
          <DialogHeader
            title={editing ? t("editTitle") : t("addTitle")}
            onOpenChange={() => onClose()}
          />
          <Stack gap={4} paddingInline={4}>
            <TextInput
              label={t("columns.pattern")}
              value={pattern}
              onChange={setPattern}
              isRequired
              hasAutoFocus
              description={t(`kindHelp.${matchType}`)}
              status={badRegex ? { type: "error", message: t("badRegex") } : undefined}
            />
            <SegmentedControl
              label={t("columns.kind")}
              value={matchType}
              onChange={(v) => setMatchType(v as typeof matchType)}
              layout="fill"
            >
              {(["word", "substring", "regex"] as const).map((k) => (
                <SegmentedControlItem key={k} value={k} label={t(`kinds.${k}`)} />
              ))}
            </SegmentedControl>
            <SegmentedControl
              label={t("columns.action")}
              value={action}
              onChange={(v) => setAction(v as typeof action)}
              layout="fill"
            >
              {(["flag", "mask", "block"] as const).map((a) => (
                <SegmentedControlItem key={a} value={a} label={t(`actions.${a}`)} />
              ))}
            </SegmentedControl>
            <CheckboxList
              label={t("columns.channels")}
              value={channels}
              onChange={(v: string[]) => setChannels(v)}
            >
              {CHANNELS.map((c) => (
                <CheckboxListItem key={c} value={c} label={tc(`channels.${c}`)} />
              ))}
            </CheckboxList>
            <TextInput
              label={t("columns.category")}
              value={category}
              onChange={setCategory}
              description={
                categories.length ? t("categoryHint", { list: categories.join(", ") }) : undefined
              }
            />
            <TextInput label={t("note")} value={note} onChange={setNote} isOptional />
            <Switch label={t("columns.enabled")} value={enabled} onChange={setEnabled} />
          </Stack>
          <Stack direction="horizontal" gap={2} justify="end" paddingInline={4} paddingBlockEnd={4}>
            <Button label={tc("cancel")} variant="ghost" onClick={onClose} />
            <Button
              label={tc("save")}
              variant="primary"
              isDisabled={!pattern.trim() || badRegex || channels.length === 0}
              isLoading={busy}
              onClick={save}
            />
          </Stack>
        </Stack>
      )}
    </Dialog>
  );
}
