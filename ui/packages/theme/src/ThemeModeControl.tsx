"use client";

import { SegmentedControl, SegmentedControlItem } from "@astryxdesign/core/SegmentedControl";
import { useThemeMode } from "./DaintyThemeProvider";
import { THEME_MODES, type ThemeMode } from "./mode";

/** Labels come from the app's catalog: the theme package holds no user-facing text. */
export function ThemeModeControl({
  label,
  labels,
  size = "sm",
}: {
  label: string;
  labels: Record<ThemeMode, string>;
  size?: "sm" | "md";
}) {
  const { mode, setMode } = useThemeMode();
  return (
    <SegmentedControl
      label={label}
      value={mode}
      onChange={(v) => setMode(v as ThemeMode)}
      size={size}
    >
      {THEME_MODES.map((m) => (
        <SegmentedControlItem key={m} value={m} label={labels[m]} />
      ))}
    </SegmentedControl>
  );
}
