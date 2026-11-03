/**
 * Colour-mode preference: what the person picked, not the mode in effect - "system" stays "system".
 * Astryx reads a missing `data-theme` as `color-scheme: light dark`, so "system" needs no JS.
 * Server-safe: no React here, so a server component may import it.
 */
export type ThemeMode = "light" | "dark" | "system";

export const THEME_MODES: readonly ThemeMode[] = ["system", "light", "dark"];

/** A cookie, not localStorage: the dashboard's server renders `<html data-theme>` from it on first paint. */
export const THEME_COOKIE = "dainty-theme";
export const THEME_COOKIE_MAX_AGE = 60 * 60 * 24 * 365;

/**
 * The accents an app can take, each one of Astryx's hues (Caddy Proxy Manager's list). Yellow and gray
 * are left out: neither holds the contrast an accent's text and controls need.
 */
export const ACCENTS = [
  "pink",
  "purple",
  "blue",
  "cyan",
  "teal",
  "green",
  "orange",
  "red",
] as const;
export type Accent = (typeof ACCENTS)[number];
export const DEFAULT_ACCENT: Accent = "pink";

export function isThemeMode(value: unknown): value is ThemeMode {
  return value === "light" || value === "dark" || value === "system";
}

export function parseThemeMode(value: string | undefined | null): ThemeMode {
  return isThemeMode(value) ? value : "system";
}

export function parseAccent(value: string | undefined | null): Accent {
  return (ACCENTS as readonly string[]).includes(value ?? "") ? (value as Accent) : DEFAULT_ACCENT;
}

/** The `data-theme` value for `<html>`, or undefined so the OS decides. */
export function themeAttr(mode: ThemeMode): "light" | "dark" | undefined {
  return mode === "system" ? undefined : mode;
}

/** `--color-background-body` in each mode and palette, for `<meta name="theme-color">`; keep in step with variants.css. */
export const BODY_COLOR = { light: "#f1f1f1", dark: "#000000" } as const;
const PALETTE_BODY: Record<string, { light: string; dark: string }> = {
  neutral: BODY_COLOR,
  slate: { light: "#eef1f5", dark: "#0b0f14" },
  midnight: { light: "#eef0fa", dark: "#070b18" },
  ember: { light: "#f6f1e9", dark: "#0d0a06" },
  forest: { light: "#eef3ef", dark: "#060b08" },
};

export function themeColor(
  mode: ThemeMode,
  palette = "neutral",
): string | { media: string; color: string }[] {
  const body = PALETTE_BODY[palette] ?? BODY_COLOR;
  if (mode !== "system") return body[mode];
  return [
    { media: "(prefers-color-scheme: light)", color: body.light },
    { media: "(prefers-color-scheme: dark)", color: body.dark },
  ];
}

/**
 * A viewer's own look, beside the colour mode: the surfaces' palette, an accent that overrides the
 * server's ("auto" keeps it), and the text size. Each palette only moves the backgrounds, so every
 * ink keeps the contrast the theme was checked at.
 */
export const PALETTES = ["neutral", "slate", "midnight", "ember", "forest"] as const;
export type Palette = (typeof PALETTES)[number];

export const TEXT_SIZES = ["default", "large"] as const;
export type TextSize = (typeof TEXT_SIZES)[number];

export type AccentChoice = Accent | "auto";

export interface Appearance {
  palette: Palette;
  accent: AccentChoice;
  textSize: TextSize;
}

export const DEFAULT_APPEARANCE: Appearance = {
  palette: "neutral",
  accent: "auto",
  textSize: "default",
};

/** Beside THEME_COOKIE, for the dashboard's first paint: `<palette>.<accent>.<text size>`. */
export const APPEARANCE_COOKIE = "dainty-appearance";

function oneOf<T extends string>(list: readonly T[], value: string | undefined, fallback: T): T {
  return (list as readonly string[]).includes(value ?? "") ? (value as T) : fallback;
}

export function parseAppearance(value: string | undefined | null): Appearance {
  const [palette, accent, textSize] = (value ?? "").split(".");
  return {
    palette: oneOf(PALETTES, palette, DEFAULT_APPEARANCE.palette),
    accent: oneOf<AccentChoice>([...ACCENTS, "auto"], accent, DEFAULT_APPEARANCE.accent),
    textSize: oneOf(TEXT_SIZES, textSize, DEFAULT_APPEARANCE.textSize),
  };
}

export function formatAppearance(a: Appearance): string {
  return `${a.palette}.${a.accent}.${a.textSize}`;
}

/** The accent in effect: the viewer's, or the server's when they left it on "auto". */
export function effectiveAccent(choice: AccentChoice, server: Accent | undefined): Accent {
  return choice === "auto" ? (server ?? DEFAULT_ACCENT) : choice;
}

/** `<html>`'s data attributes for a look, so a server render and the client agree. */
export function appearanceAttrs(a: Appearance, server: Accent | undefined) {
  return {
    "data-dainty-accent": effectiveAccent(a.accent, server),
    "data-dainty-palette": a.palette === "neutral" ? undefined : a.palette,
    "data-dainty-text": a.textSize === "default" ? undefined : a.textSize,
  };
}
