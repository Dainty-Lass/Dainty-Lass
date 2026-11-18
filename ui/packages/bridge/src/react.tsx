"use client";

import { Stack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { createContext, type ReactNode, use, useEffect, useState } from "react";
import type { HostKit, HostServerView, Snapshot, TaskProgress } from "./generated/types";
import { getLauncher, inTauri } from "./index";
import type { LauncherApi } from "./types";

interface BridgeState {
  api: LauncherApi | null;
  snapshot: Snapshot | null;
  progress: TaskProgress | null;
  kit: HostKit | null;
  server: HostServerView | null;
  /** The game console's lines this game (the launcher's; empty in the server manager). */
  gameLog: string[];
}

const BridgeContext = createContext<BridgeState>({
  api: null,
  snapshot: null,
  progress: null,
  kit: null,
  server: null,
  gameLog: [],
});

/** As many as the launcher keeps (platform/game_console.rs). */
const KEPT_LINES = 5000;

/**
 * Holds what the backend pushes: the snapshot on every change, task progress, and for the server manager
 * the host kit and the server's state. `host` subscribes to the last two only where they are shown.
 */
export function BridgeProvider({
  host = false,
  children,
}: {
  host?: boolean;
  children: ReactNode;
}) {
  const [state, setState] = useState<BridgeState>({
    api: null,
    snapshot: null,
    progress: null,
    kit: null,
    server: null,
    gameLog: [],
  });

  useEffect(() => {
    let live = true;
    void getLauncher().then(async (api) => {
      if (!live) return;
      // A new game starts a new log, as the launcher's own does
      api.onSnapshot((snapshot) =>
        setState((s) => ({
          ...s,
          snapshot,
          gameLog: snapshot.running.game && !s.snapshot?.running.game ? [] : s.gameLog,
        })),
      );
      api.onProgress((progress) => setState((s) => ({ ...s, progress })));
      const snapshot = await api.getSnapshot();
      setState((s) => ({ ...s, api, snapshot }));
      if (!host) {
        api.onGameLog((lines) =>
          setState((s) => ({ ...s, gameLog: [...s.gameLog, ...lines].slice(-KEPT_LINES) })),
        );
        const gameLog = await api.gameLogGet();
        setState((s) => ({ ...s, gameLog }));
      }
      if (host) {
        api.onHostKit((kit) => setState((s) => ({ ...s, kit })));
        api.onHostServer((server) => setState((s) => ({ ...s, server })));
        const [kit, server] = await Promise.all([api.hostGet(), api.hostServerGet()]);
        setState((s) => ({ ...s, kit, server }));
      }
    });
    return () => {
      live = false;
    };
  }, [host]);

  return <BridgeContext value={state}>{children}</BridgeContext>;
}

export function useBridge(): BridgeState {
  return use(BridgeContext);
}

/** A Windows 11 caption button (dainty-caption in brand.css), with Segoe Fluent's 10px, 1px glyphs. */
function CaptionButton({
  label,
  d,
  close = false,
  onClick,
}: {
  label: string;
  d: string;
  close?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={close ? "dainty-caption dainty-caption-close" : "dainty-caption"}
      aria-label={label}
      title={label}
      onClick={onClick}
    >
      <svg
        width="10"
        height="10"
        viewBox="0 0 10 10"
        fill="none"
        stroke="currentColor"
        aria-hidden="true"
      >
        <path d={d} />
      </svg>
    </button>
  );
}

/**
 * The frameless window's own title bar (the Tauri window has decorations off): the whole bar drags the
 * window, and the three buttons opt out of dragging (dainty-titlebar in brand.css).
 */
export function TitleBar({
  title,
  start,
  labels,
}: {
  title: string;
  start?: ReactNode;
  labels: { minimize: string; maximize: string; close: string };
}) {
  const { api } = useBridge();
  const [maximized, setMaximized] = useState(false);
  useEffect(() => api?.onWindowState(setMaximized), [api]);
  return (
    <div className="dainty-titlebar" data-tauri-drag-region>
      <Stack
        direction="horizontal"
        align="center"
        justify="between"
        paddingInlineStart={3}
        height={36}
      >
        <Stack direction="horizontal" gap={2} align="center">
          {start}
          <Text type="label" color="secondary">
            {title}
          </Text>
        </Stack>
        {inTauri() && (
          <Stack direction="horizontal" gap={0}>
            <CaptionButton
              label={labels.minimize}
              d="M0 5.5h10"
              onClick={() => void api?.minimize()}
            />
            <CaptionButton
              label={labels.maximize}
              d={maximized ? "M2.5 2.5V.5h7v7h-2M.5 2.5h7v7h-7z" : "M.5.5h9v9h-9z"}
              onClick={async () => setMaximized((await api?.toggleMaximize()) ?? false)}
            />
            <CaptionButton
              label={labels.close}
              d="M.5.5l9 9M9.5.5l-9 9"
              close
              onClick={() => void api?.close()}
            />
          </Stack>
        )}
      </Stack>
    </div>
  );
}
