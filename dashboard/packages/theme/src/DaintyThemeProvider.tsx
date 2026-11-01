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
  parseThemeMode,
  THEME_COOKIE,
  THEME_COOKIE_MAX_AGE,
  type ThemeMode,
} from "./mode";

interface ThemeModeContextValue {
  mode: ThemeMode;
  setMode: (mode: ThemeMode) => void;
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

function persist(mode: ThemeMode, where: ThemePersistence) {
  if (where === "cookie") {
    /* biome-ignore lint/suspicious/noDocumentCookie: Cookie Store is Chromium-only and async. */
    document.cookie = `${THEME_COOKIE}=${mode}; path=/; max-age=${THEME_COOKIE_MAX_AGE}; SameSite=Lax`;
    return;
  }
  try {
    localStorage.setItem(LOCAL_KEY, mode);
  } catch {
    // Storage refused (private mode): the choice lasts this session only.
  }
}

/**
 * The one theme wrapper for every Dainty Lass app. `initialMode` comes from what the server read, so the
 * first client render matches `<html data-theme>`; static pages pass nothing and read local storage.
 */
export function DaintyThemeProvider({
  initialMode,
  accent,
  persistence = "cookie",
  children,
}: {
  initialMode?: ThemeMode;
  /** Mirrored onto <html data-dainty-accent> for the pages that cannot render it from the server. */
  accent?: Accent;
  persistence?: ThemePersistence;
  children: ReactNode;
}) {
  const [mode, setModeState] = useState<ThemeMode>(initialMode ?? "system");

  useEffect(() => {
    if (initialMode !== undefined || persistence !== "local") return;
    try {
      setModeState(parseThemeMode(localStorage.getItem(LOCAL_KEY)));
    } catch {
      // No storage: follow the OS.
    }
  }, [initialMode, persistence]);

  useEffect(() => {
    if (accent === undefined) return;
    document.documentElement.dataset.daintyAccent = accent;
  }, [accent]);

  const setMode = useCallback(
    (next: ThemeMode) => {
      setModeState(next);
      persist(next, persistence);
    },
    [persistence],
  );

  const value = useMemo(() => ({ mode, setMode }), [mode, setMode]);

  return (
    <ThemeModeContext value={value}>
      <Theme theme={daintyTheme} mode={mode}>
        {children}
      </Theme>
    </ThemeModeContext>
  );
}
