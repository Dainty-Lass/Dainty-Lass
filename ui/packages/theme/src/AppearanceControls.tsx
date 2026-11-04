"use client";

import { Grid } from "@astryxdesign/core/Grid";
import { SegmentedControl, SegmentedControlItem } from "@astryxdesign/core/SegmentedControl";
import { SelectableCard } from "@astryxdesign/core/SelectableCard";
import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { HUES } from "./accent";
import { useThemeMode } from "./DaintyThemeProvider";
import {
  ACCENTS,
  type Accent,
  type AccentChoice,
  PALETTES,
  type Palette,
  TEXT_SIZES,
  type TextSize,
} from "./mode";
import { ThemeModeControl } from "./ThemeModeControl";

export interface AppearanceLabels {
  mode: string;
  modes: Record<"system" | "light" | "dark", string>;
  palette: string;
  palettes: Record<Palette, string>;
  accent: string;
  /** "auto": the server's accent, named so the viewer knows which it is. */
  accentAuto: string;
  accents: Record<Accent, string>;
  textSize: string;
  textSizes: Record<TextSize, string>;
}

/**
 * A viewer's whole look in one block (Caddy Proxy Manager's AccentColorPicker, as cards): the mode,
 * the surfaces' palette, the accent and the text size. Applied as picked; nothing to save. Labels
 * come from the app's catalog: the theme package holds no user-facing text.
 */
export function AppearanceControls({ labels }: { labels: AppearanceLabels }) {
  const { appearance, setAppearance, serverAccent } = useThemeMode();
  const accentCard = (choice: AccentChoice, label: string) => {
    const hue = choice === "auto" ? (serverAccent ?? "pink") : choice;
    return (
      <SelectableCard
        key={choice}
        label={label}
        variant={appearance.accent === choice ? hue : "default"}
        isSelected={appearance.accent === choice}
        onChange={() => setAppearance({ accent: choice })}
        padding={3}
      >
        <Stack direction="horizontal" gap={2} align="center">
          <span aria-hidden className={`dainty-swatch ${HUES[hue].text}`} />
          <Text>{label}</Text>
        </Stack>
      </SelectableCard>
    );
  };

  return (
    <Stack gap={5}>
      <Stack gap={2}>
        <Text weight="medium">{labels.mode}</Text>
        <ThemeModeControl label={labels.mode} labels={labels.modes} size="md" />
      </Stack>

      <Stack gap={2}>
        <Text weight="medium">{labels.palette}</Text>
        <Grid columns={{ minWidth: 140, max: 5 }} gap={2}>
          {PALETTES.map((p) => (
            <SelectableCard
              key={p}
              label={labels.palettes[p]}
              isSelected={appearance.palette === p}
              onChange={() => setAppearance({ palette: p })}
              padding={3}
            >
              <Stack gap={2}>
                {/* The palette's rail, page and card, in the mode in effect */}
                <span className="dainty-palette-preview" data-palette={p} aria-hidden>
                  <span />
                  <span />
                </span>
                <Text>{labels.palettes[p]}</Text>
              </Stack>
            </SelectableCard>
          ))}
        </Grid>
      </Stack>

      <Stack gap={2}>
        <Text weight="medium">{labels.accent}</Text>
        <Grid columns={{ minWidth: 120, max: 3 }} gap={2}>
          {accentCard("auto", labels.accentAuto)}
          {ACCENTS.map((a) => accentCard(a, labels.accents[a]))}
        </Grid>
      </Stack>

      <Stack gap={2}>
        <Text weight="medium">{labels.textSize}</Text>
        <SegmentedControl
          label={labels.textSize}
          value={appearance.textSize}
          onChange={(v) => setAppearance({ textSize: v as TextSize })}
          size="md"
        >
          {TEXT_SIZES.map((s) => (
            <SegmentedControlItem key={s} value={s} label={labels.textSizes[s]} />
          ))}
        </SegmentedControl>
      </Stack>
    </Stack>
  );
}
