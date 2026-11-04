"use client";

import { TextInput } from "@astryxdesign/core/TextInput";
import { useEffect, useState } from "react";

/** A search box that commits on Enter or clear, not on every key, since each commit is a server render. */
export function SearchField({
  label,
  value,
  placeholder,
  onCommit,
  width = 280,
}: {
  label: string;
  value: string;
  placeholder?: string;
  onCommit: (value: string) => void;
  width?: number;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return (
    <TextInput
      label={label}
      isLabelHidden
      value={draft}
      placeholder={placeholder}
      startIcon="search"
      hasClear
      width={width}
      size="sm"
      onChange={(v) => {
        setDraft(v);
        if (v === "" && value !== "") onCommit("");
      }}
      onEnter={() => onCommit(draft.trim())}
    />
  );
}
