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

/** `--color-background-body` in each mode, for `<meta name="theme-color">`; keep in step with variants.css. */
export const BODY_COLOR = { light: "#f1f1f1", dark: "#000000" } as const;

export function themeColor(mode: ThemeMode): string | { media: string; color: string }[] {
  if (mode !== "system") return BODY_COLOR[mode];
  return [
    { media: "(prefers-color-scheme: light)", color: BODY_COLOR.light },
    { media: "(prefers-color-scheme: dark)", color: BODY_COLOR.dark },
  ];
}
