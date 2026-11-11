"use client";

import type { HostKit, LauncherApi } from "@dainty/bridge";
import { useBridge } from "@dainty/bridge/react";
import { useEffect, useSyncExternalStore } from "react";

// The launcher's BridgeProvider leaves the host kit to the manager, so the Host page subscribes here, once:
// the bridge's listeners cannot be removed.
let kit: HostKit | null = null;
let started = false;
const listeners = new Set<() => void>();

function set(next: HostKit) {
  kit = next;
  for (const fn of listeners) fn();
}

function start(api: LauncherApi) {
  if (started) return;
  started = true;
  api.onHostKit(set);
  void refreshHostKit(api);
}

/** Runs the requirement checks again and answers the kit they produce. */
export async function refreshHostKit(api: LauncherApi) {
  try {
    set(await api.hostGet());
  } catch (error) {
    console.error(error);
  }
}

function subscribe(fn: () => void) {
  listeners.add(fn);
  return () => void listeners.delete(fn);
}

export function useHostKit(): HostKit | null {
  const { api } = useBridge();
  useEffect(() => {
    if (api) start(api);
  }, [api]);
  return useSyncExternalStore(
    subscribe,
    () => kit,
    () => null,
  );
}
