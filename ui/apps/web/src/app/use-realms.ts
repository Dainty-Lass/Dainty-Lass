// The Realms screen's state (DU5, RF7): the transport's availability, the active session and the tracked local
// sessions, over the bridge's realm_* commands.
import { useCallback, useEffect, useState } from "react"

import {
  api,
  type RealmKind,
  type RealmOutcome,
  type RealmView,
} from "@workspace/bridge"

import { useLauncher } from "./launcher"

const SESSION_POLL_MS = 2000

export type RealmWorking =
  "refresh" | "host_hub" | "host_hunt" | "join" | "stop"

export interface RealmsState {
  outcome: RealmOutcome | null
  realm: RealmView | null
  working: RealmWorking | null
  refresh(): Promise<void>
  host(kind: RealmKind): Promise<boolean>
  join(realmId: string): Promise<boolean>
  stop(): Promise<boolean>
}

export function useRealms(): RealmsState {
  const { realm: outcome, setRealm, report } = useLauncher()
  const [working, setWorking] = useState<RealmWorking | null>(null)
  const realm = outcome?.ok ? outcome.realm : null

  const pull = useCallback(async (): Promise<RealmOutcome> => {
    const next = await api.realmGet()
    setRealm(next)
    return next
  }, [setRealm])

  useEffect(() => {
    void pull()
  }, [pull])

  const live = Boolean(
    realm?.sessions.some((s) => s.state !== "ended") ||
    (realm?.active && realm.active.state !== "ended")
  )
  useEffect(() => {
    if (!live) return
    const timer = setInterval(() => {
      void pull()
      void api.refreshStatus()
    }, SESSION_POLL_MS)
    return () => clearInterval(timer)
  }, [live, pull])

  const guard = useCallback(
    async (
      what: RealmWorking,
      run: () => Promise<RealmOutcome>
    ): Promise<boolean> => {
      setWorking(what)
      try {
        const next = await run()
        setRealm(next)
        if (!next.ok) {
          await report(next)
          return false
        }
        return true
      } finally {
        setWorking(null)
      }
    },
    [setRealm, report]
  )

  const refresh = useCallback(async () => {
    setWorking("refresh")
    try {
      await pull()
      void api.refreshStatus()
    } finally {
      setWorking(null)
    }
  }, [pull])

  const host = useCallback(
    (kind: RealmKind) =>
      guard(kind === "hunt" ? "host_hunt" : "host_hub", () =>
        api.realmHost(kind)
      ),
    [guard]
  )

  const join = useCallback(
    (realmId: string) => guard("join", () => api.realmJoin(realmId.trim())),
    [guard]
  )

  const stop = useCallback(() => guard("stop", () => api.realmStop()), [guard])

  return {
    outcome,
    realm,
    working,
    refresh,
    host,
    join,
    stop,
  }
}
