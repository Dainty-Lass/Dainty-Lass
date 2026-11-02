// What the Realms screen shows and offers (DU5, RF7, DD11, DD14), as pure functions over the bridge's types,
// so the decisions are tested without a DOM (realms.test.ts). Every string is a catalog key.
import type {
  ModuleView,
  RealmKind,
  RealmOutcome,
  RealmSessionView,
  RealmState,
  Snapshot,
} from "@workspace/bridge"
import { isKey, type Key } from "@workspace/i18n"

import type { Tone } from "./host-model"
import type { Message } from "./model"

// Matches REALM_ID_MAX in rust/launcher/src-tauri/src/bridge/spec.rs
export const REALM_ID_MAX = 128

// Whether the launcher build serves the realm_* commands (`realms` Cargo feature, RF7)
export function realmsInBuild(outcome: RealmOutcome | null): boolean {
  if (outcome === null) return false
  if (outcome.ok) return true
  return (
    outcome.error.code !== "not_in_this_build" &&
    outcome.error.code !== "not_implemented"
  )
}

// Realms exist on 1.14.7 (modules with the realms capability, DD11, DD14), never on 1.4.4 or 1.12.0
export function realmsSupported(
  snap: Snapshot | null,
  modules?: ModuleView[] | null
): boolean {
  if (snap === null) return false
  if (
    snap.settings.gameEdition === "legacy" ||
    snap.revived !== null ||
    snap.server?.game === "revived"
  ) {
    return false
  }
  const build = snap.status?.game?.build ?? null
  if (build === "1.12.0" || build === "1.4.4") return false
  if (modules && modules.length > 0) {
    const loadedGames = modules.filter(
      (m) => m.role === "game" && m.state === "ready"
    )
    if (loadedGames.length > 0) {
      return loadedGames.some((m) => m.capabilities.realms)
    }
  }
  return true
}

export function realmsPageAvailable(
  snap: Snapshot | null,
  outcome: RealmOutcome | null,
  modules?: ModuleView[] | null
): boolean {
  return realmsInBuild(outcome) && realmsSupported(snap, modules)
}

export type HubTarget = "ramsgate" | "training"

export const HUB_TARGET_KEYS: Record<HubTarget, Key> = {
  ramsgate: "rlm_hub_ramsgate",
  training: "rlm_hub_training",
}

export const REALM_KIND_KEYS: Record<RealmKind, Key> = {
  hub: "rlm_kind_hub",
  hunt: "rlm_kind_hunt",
}

export const REALM_STATE_KEYS: Record<RealmState, Key> = {
  starting: "rlm_state_starting",
  ready: "rlm_state_ready",
  running: "rlm_state_running",
  ended: "rlm_state_ended",
}

export function realmStateTone(state: RealmState, result: string | null): Tone {
  switch (state) {
    case "ready":
    case "running":
      return "good"
    case "starting":
      return "busy"
    case "ended":
      return result === null || result === "clean" ? "idle" : "bad"
  }
}

export function sessionRoleKey(
  session: Pick<RealmSessionView, "kind" | "hosting">
): Key {
  if (session.hosting) {
    return session.kind === "hunt" ? "rlm_role_host_hunt" : "rlm_role_host_hub"
  }
  return session.kind === "hunt" ? "rlm_role_join_hunt" : "rlm_role_join_hub"
}

export function sessionResultMessage(result: string | null): Message | null {
  if (result === null) return null
  const key = `rlm_result_${result}`
  return { key: isKey(key) ? key : "rlm_result_ended" }
}

// What the session's local UDP route and the launcher's gateway relay are doing right now
export function relayStatusMessage(
  session: RealmSessionView,
  gatewayRelayPort: number | null
): Message {
  if (session.state === "ended") {
    return sessionResultMessage(session.result) ?? { key: "rlm_result_ended" }
  }
  if (session.localPort !== null) {
    if (gatewayRelayPort !== null) {
      return {
        key: "rlm_relay_both",
        vars: { port: session.localPort, gateway: gatewayRelayPort },
      }
    }
    return { key: "rlm_relay_ready", vars: { port: session.localPort } }
  }
  return { key: "rlm_relay_starting" }
}

export type RealmIdCheck = "empty" | "too_long" | "bad_chars" | "ok"

export function checkRealmId(input: unknown): RealmIdCheck {
  if (typeof input !== "string") return "empty"
  const trimmed = input.trim()
  if (trimmed.length === 0) return "empty"
  if (trimmed.length > REALM_ID_MAX) return "too_long"
  // Matches realm_id in rust/launcher/src-tauri/src/bridge/commands.rs: no whitespace or control characters
  for (let i = 0; i < trimmed.length; i++) {
    const code = trimmed.charCodeAt(i)
    if (code <= 0x20 || code === 0x7f) return "bad_chars"
  }
  return "ok"
}
