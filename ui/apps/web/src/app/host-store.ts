// The Host page's state: what the host kit found on this PC and how the server is, pushed by the backend
// (`host-kit`, `host-server`). The bridge's listeners can't be removed, so they are added once, on the first
// look at the Host page, and the screens read the latest value.
import { useCallback, useSyncExternalStore } from "react"

import {
  api,
  inLauncher,
  type ActionResult,
  type HostKit,
  type HostServerView,
} from "@workspace/bridge"

import { useLauncher } from "@/app/launcher"

interface Held {
  kit: HostKit | null
  server: HostServerView | null
}

let held: Held = { kit: null, server: null }
let started = false
const listeners = new Set<() => void>()

function set(next: Partial<Held>): void {
  held = { ...held, ...next }
  listeners.forEach((listener) => listener())
}

// Looks the kit up again, for "Check again" and after the player installed something
export async function refreshKit(): Promise<void> {
  if (!inLauncher()) return
  set({ kit: await api.hostGet() })
}

export function startHostStore(): void {
  if (started || !inLauncher()) return
  started = true
  api.onHostKit((kit) => set({ kit }))
  api.onHostServer((server) => set({ server }))
  void refreshKit().catch((error: unknown) => console.error(error))
  void api.hostServerGet().then(
    (server) => set({ server }),
    (error: unknown) => console.error(error)
  )
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function useHost(): Held {
  return useSyncExternalStore(
    subscribe,
    () => held,
    () => held
  )
}

// An action's answer goes to the banner when the backend didn't keep its error
export function useAct(): (call: () => Promise<ActionResult>) => void {
  const { report } = useLauncher()
  return useCallback((call) => void call().then(report), [report])
}
