"use client";

import { Selector } from "@astryxdesign/core/Selector";
import { Stack } from "@astryxdesign/core/Stack";
import { TextArea } from "@astryxdesign/core/TextArea";
import { useTranslations } from "next-intl";
import { useState } from "react";

const CUSTOM = "__custom";

/** Pick a stock reason or write one; the stock list is Settings → Moderation reasons. */
export function ReasonField({
  presets,
  value,
  onChange,
  label,
  isOptional,
}: {
  presets: string[];
  value: string;
  onChange: (value: string) => void;
  label: string;
  isOptional?: boolean;
}) {
  const t = useTranslations("moderation");
  const [choice, setChoice] = useState<string>(
    presets.includes(value) ? value : value ? CUSTOM : "",
  );
  return (
    <Stack gap={2}>
      <Selector
        label={label}
        isOptional={isOptional}
        isRequired={!isOptional}
        placeholder={t("pickReason")}
        value={choice || undefined}
        options={[
          ...presets.map((p) => ({ value: p, label: p })),
          { type: "divider" as const },
          { value: CUSTOM, label: t("customReason") },
        ]}
        onChange={(v) => {
          setChoice(v);
          onChange(v === CUSTOM ? "" : v);
        }}
        width="100%"
      />
      {choice === CUSTOM && (
        <TextArea
          label={t("customReason")}
          isLabelHidden
          value={value}
          onChange={onChange}
          rows={3}
          maxLength={400}
          placeholder={t("customPlaceholder")}
          hasAutoFocus
        />
      )}
    </Stack>
  );
}
