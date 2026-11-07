"use client";

import { Theme } from "@astryxdesign/core";
import {
  createContext,
  type ReactNode,
  use,
  useCallback,
  useEffect,
  useMemo,
  useState,
} from "react";
import { daintyTheme } from "../built/dainty.js";
import {
  type Accent,
  APPEARANCE_COOKIE,
  type Appearance,
  appearanceAttrs,
  DEFAULT_APPEARANCE,
  formatAppearance,
  parseAppearance,
  parseThemeMode,
  THEME_COOKIE,
  THEME_COOKIE_MAX_AGE,
  type ThemeMode,
} from "./mode";

interface ThemeModeContextValue {
  mode: ThemeMode;
  setMode: (mode: ThemeMode) => void;
  appearance: Appearance;
  setAppearance: (patch: Partial<Appearance>) => void;
  /** The server's accent, which `accent: "auto"` follows. */
  serverAccent: Accent | undefined;
}

const ThemeModeContext = createContext<ThemeModeContextValue | null>(null);

export function useThemeMode(): ThemeModeContextValue {
  const ctx = use(ThemeModeContext);
  if (!ctx) throw new Error("useThemeMode must be used inside <DaintyThemeProvider>");
  return ctx;
}

/**
 * Where the preference lives: a cookie for the dashboard (its server renders the first paint), local
 * storage for the Tauri pages, which are static files with no server to read a cookie.
 */
export type ThemePersistence = "cookie" | "local";

const LOCAL_KEY = "dainty-theme";
const LOCAL_APPEARANCE_KEY = "dainty-appearance";

function persist(name: string, value: string, where: ThemePersistence) {
  if (where === "cookie") {
    /* biome-ignore lint/suspicious/noDocumentCookie: Cookie Store is Chromium-only and async. */
    document.cookie = `${name}=${value}; path=/; max-age=${THEME_COOKIE_MAX_AGE}; SameSite=Lax`;
    return;
  }
  try {
    localStorage.setItem(name === THEME_COOKIE ? LOCAL_KEY : LOCAL_APPEARANCE_KEY, value);
  } catch {
    // Storage refused (private mode): the choice lasts this session only.
  }
}

function setAttr(name: string, value: string | undefined) {
  const root = document.documentElement;
  if (value === undefined) root.removeAttribute(name);
  else root.setAttribute(name, value);
}

/**
 * The one theme wrapper for every Dainty Lass app. `initialMode` and `initialAppearance` come from what
 * the server read, so the first client render matches `<html>`; static pages pass neither and read
 * local storage. `accent` is the server's (Settings, or a server's launcher branding).
 */
export function DaintyThemeProvider({
  initialMode,
  initialAppearance,
  accent,
  persistence = "cookie",
  children,
}: {
  initialMode?: ThemeMode;
  initialAppearance?: Appearance;
  accent?: Accent;
  persistence?: ThemePersistence;
  children: ReactNode;
}) {
  const [mode, setModeState] = useState<ThemeMode>(initialMode ?? "system");
  const [appearance, setAppearanceState] = useState<Appearance>(
    initialAppearance ?? DEFAULT_APPEARANCE,
  );

  useEffect(() => {
    if (persistence !== "local") return;
    try {
      if (initialMode === undefined) setModeState(parseThemeMode(localStorage.getItem(LOCAL_KEY)));
      if (initialAppearance === undefined)
        setAppearanceState(parseAppearance(localStorage.getItem(LOCAL_APPEARANCE_KEY)));
    } catch {
      // No storage: the defaults.
    }
  }, [initialMode, initialAppearance, persistence]);

  // Mirrored onto <html>, where variants.css reads them, for every page and every change.
  useEffect(() => {
    for (const [name, value] of Object.entries(appearanceAttrs(appearance, accent)))
      setAttr(name, value);
  }, [appearance, accent]);

  const setMode = useCallback(
    (next: ThemeMode) => {
      setModeState(next);
      persist(THEME_COOKIE, next, persistence);
    },
    [persistence],
  );

  const setAppearance = useCallback(
    (patch: Partial<Appearance>) => {
      setAppearanceState((current) => {
        const next = { ...current, ...patch };
        persist(APPEARANCE_COOKIE, formatAppearance(next), persistence);
        return next;
      });
    },
    [persistence],
  );

  const value = useMemo(
    () => ({ mode, setMode, appearance, setAppearance, serverAccent: accent }),
    [mode, setMode, appearance, setAppearance, accent],
  );

  return (
    <ThemeModeContext value={value}>
      <Theme theme={daintyTheme} mode={mode}>
        {children}
      </Theme>
    </ThemeModeContext>
  );
}
