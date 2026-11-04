"use client";

import { ACCENTS, AppearanceControls, PALETTES, TEXT_SIZES } from "@dainty/theme";
import { useTranslations } from "next-intl";

/** The theme package's Appearance block in this app's words, for every mode (`appearance` catalog). */
export function AppearanceSettings() {
  const t = useTranslations("appearance");
  return (
    <AppearanceControls
      labels={{
        mode: t("mode"),
        modes: { system: t("modes.system"), light: t("modes.light"), dark: t("modes.dark") },
        palette: t("palette"),
        palettes: Object.fromEntries(PALETTES.map((p) => [p, t(`palettes.${p}`)])) as Record<
          (typeof PALETTES)[number],
          string
        >,
        accent: t("accent"),
        accentAuto: t("accentAuto"),
        accents: Object.fromEntries(ACCENTS.map((a) => [a, t(`accents.${a}`)])) as Record<
          (typeof ACCENTS)[number],
          string
        >,
        textSize: t("textSize"),
        textSizes: Object.fromEntries(TEXT_SIZES.map((s) => [s, t(`textSizes.${s}`)])) as Record<
          (typeof TEXT_SIZES)[number],
          string
        >,
      }}
    />
  );
}
