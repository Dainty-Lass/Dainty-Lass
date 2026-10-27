// What the screens show for a Snapshot, as pure functions over the bridge's types, so the decisions are
// tested without a DOM (model.test.ts). Every string is a catalog key.
import type {
  ErrorCode,
  GameSource,
  GraphicsPreset,
  InstanceKind,
  LauncherError,
  NoticeCode,
  Phase,
  PlayerWhere,
  RuntimeState,
  ServerMode,
  Snapshot,
  StatusInstance,
  TaskProgress,
} from "@workspace/bridge"
import { CATALOGS, isKey, type Key, type Language } from "@workspace/i18n"

import {
  formatBytes,
  formatDuration,
  formatPercent,
  formatSpeed,
} from "./format"

// A sentence and what fills it. A var may be another key, translated in place; `detail` is a technical
// note (a path, a Windows error) shown under the sentence, never spliced into it.
export type Var = string | number | { key: Key }
export interface Message {
  key: Key
  vars?: Record<string, Var>
  detail?: string
}

// One key per error code; the type makes a code the backend adds without text a typecheck failure
export const ERROR_KEYS: Record<ErrorCode, Key> = {
  account_banned: "err_account_banned",
  invite_invalid_format: "err_invite_invalid_format",
  invite_invalid: "err_invite_invalid",
  username_invalid: "err_username_invalid",
  username_taken: "err_username_taken",
  registration_closed: "err_registration_closed",
  bad_request: "err_bad_request",
  server_unreachable: "err_server_unreachable",
  server_error: "err_server_error",
  key_invalid: "err_key_invalid",
  key_rejected: "err_key_rejected",
  key_storage_unavailable: "err_key_storage_unavailable",
  folder_invalid: "err_folder_invalid",
  game_folder_not_found: "err_game_folder_not_found",
  game_wrong_version: "err_game_wrong_version",
  runtime_unavailable: "err_runtime_unavailable",
  runtime_untrusted: "err_runtime_untrusted",
  runtime_failed: "err_runtime_failed",
  runtime_different_on_server: "err_runtime_different_on_server",
  runtime_in_use: "err_runtime_in_use",
  local_runtime_invalid: "err_local_runtime_invalid",
  download_failed: "err_download_failed",
  disk_space: "err_disk_space",
  already_running: "err_already_running",
  launch_failed: "err_launch_failed",
  compat_runtime_missing: "err_compat_runtime_missing",
  config_failed: "err_config_failed",
  busy: "err_busy",
  cancelled: "err_cancelled",
  cert_mismatch: "err_cert_mismatch",
  cert_changed: "err_cert_changed",
  rate_limited: "err_rate_limited",
  relay_port_busy: "err_relay_port_busy",
  relay_failed: "err_relay_failed",
  loader_leftovers: "err_loader_leftovers",
  host_failed: "err_host_failed",
  manifest_mismatch: "err_manifest_mismatch",
  no_content_server: "err_no_content_server",
  file_missing_on_server: "err_file_missing_on_server",
  file_different_on_server: "err_file_different_on_server",
  dll_failed: "err_dll_failed",
  game_files_invalid: "err_game_files_invalid",
  windows_runtime_missing: "err_windows_runtime_missing",
  folder_other_game: "err_folder_other_game",
  folder_not_empty: "err_folder_not_empty",
  edition_other: "err_edition_other",
  not_implemented: "err_not_implemented",
  not_in_this_build: "err_not_in_this_build",
  epic_signed_out: "err_epic_signed_out",
  epic_library_failed: "err_epic_library_failed",
  epic_not_owned: "err_epic_not_owned",
  epic_code_invalid: "err_epic_code_invalid",
  epic_rejected: "err_epic_rejected",
  epic_unreachable: "err_epic_unreachable",
  epic_failed: "err_epic_failed",
  game_version_unknown: "err_game_version_unknown",
  game_version_unavailable: "err_game_version_unavailable",
  epic_manifest_unavailable: "err_epic_manifest_unavailable",
  guarded_not_found: "err_guarded_not_found",
  guarded_not_waiting: "err_guarded_not_waiting",
  server_stopped: "err_server_stopped",
  module_unknown: "err_module_unknown",
  module_bad_state: "err_module_bad_state",
  module_input_invalid: "err_module_input_invalid",
  module_settings_invalid: "err_module_settings_invalid",
  module_failed: "err_module_failed",
  history_bad_filter: "err_history_bad_filter",
  history_unavailable: "err_history_unavailable",
  history_plan_changed: "err_history_plan_changed",
  history_failed: "err_history_failed",
  unknown: "err_unknown",
}

export const NOTICE_KEYS: Record<NoticeCode, Key> = {
  keep_open: "notice_keep_open",
  backup_saved: "notice_backup_saved",
  runtime_installed: "notice_runtime_installed",
  runtime_installed_restart: "notice_runtime_installed_restart",
  game_verified: "notice_game_verified",
  sdk_imported: "notice_sdk_imported",
  host_done: "notice_host_done",
  install_done: "notice_install_done",
  repair_done: "notice_repair_done",
}

function keyOr(candidate: string, fallback: Key): Key {
  return isKey(candidate) ? candidate : fallback
}

// A module's refusal comes as `reason` or `reason:name`: one sentence per reason, the name filling it. A
// reason without a sentence, or one that needs a name the backend couldn't give, gets the code's own.
function reasoned(prefix: string, fallback: Key, detail: string): Message {
  const [reason = "", ...rest] = detail.split(":")
  const name = rest.join(":")
  const key = `${prefix}${reason}`
  if (!isKey(key) || (CATALOGS.en[key].includes("{name}") && name === "")) {
    return { key: fallback }
  }
  return { key, vars: { name } }
}

export function errorMessage(
  e: LauncherError,
  mode: ServerMode | null
): Message {
  const detail = e.detail ?? ""
  switch (e.code) {
    case "account_banned":
      return {
        key: "err_account_banned",
        vars: { reason: detail || { key: "err_account_banned_noreason" } },
      }
    case "invite_invalid_format":
      return {
        key: detail
          ? keyOr(`invite_err_${detail}`, "err_invite_invalid_format")
          : "err_invite_invalid_format",
      }
    case "username_invalid":
      return {
        key:
          detail && detail !== "ok"
            ? keyOr(`reg_check_${detail}`, "err_username_invalid")
            : "err_username_invalid",
      }
    case "relay_port_busy":
      return {
        key: "err_relay_port_busy",
        vars: { port: /^\d+$/.test(detail) ? detail : "61100" },
      }
    case "server_unreachable":
      return {
        key:
          mode === "public"
            ? "err_server_unreachable_public"
            : "err_server_unreachable",
      }
    case "loader_leftovers":
      return { key: "err_loader_leftovers", vars: { detail } }
    case "history_failed":
      return { key: "err_history_failed", vars: { detail } }
    case "host_failed":
      return isKey(`host_err_${detail}`)
        ? { key: `host_err_${detail}` as Key }
        : { key: "err_host_failed", vars: { detail } }
    case "module_bad_state":
      return {
        key:
          detail === "loaded" || detail === "not_loaded"
            ? (`mod_err_${detail}` as Key)
            : "err_module_bad_state",
      }
    case "module_input_invalid":
      return reasoned("mod_err_input_", "err_module_input_invalid", detail)
    case "module_settings_invalid":
      return reasoned(
        "mod_err_settings_",
        "err_module_settings_invalid",
        detail
      )
    case "module_failed":
      return isKey(`mod_err_failed_${detail}`)
        ? { key: `mod_err_failed_${detail}` as Key }
        : { key: "err_module_failed", vars: { detail } }
    case "launch_failed":
    case "windows_runtime_missing":
      return { key: ERROR_KEYS[e.code], detail: detail || undefined }
    default:
      return { key: ERROR_KEYS[e.code] }
  }
}

// The error the banner shows: the backend's, else the one an action answered that the backend didn't keep.
// key_rejected while registering has its own card instead.
export function shownError(
  s: Snapshot | null,
  local: LauncherError | null
): LauncherError | null {
  const last = s?.lastError
  if (
    last &&
    last.code !== "cancelled" &&
    !(s?.phase === "register" && last.code === "key_rejected")
  )
    return last
  return local && local.code !== "cancelled" ? local : null
}

export type PrimaryAction =
  | "join"
  | "retry"
  | "new_invite"
  | "register"
  | "locate"
  | "verify"
  | "trust"
  | "install_runtime"
  | "revived_install"
  | "play"
  | "cancel"
  | "none"

export interface PrimaryButton {
  key: Key
  action: PrimaryAction
  disabled: boolean
}

export interface FormState {
  inviteValid: boolean
  usernameValid: boolean
}

export function primaryButton(s: Snapshot, form: FormState): PrimaryButton {
  const busy = s.busy
  switch (s.phase) {
    case "loading":
      return { key: "btn_wait", action: "none", disabled: true }
    case "join":
      return {
        key: busy ? "btn_wait" : "btn_join",
        action: "join",
        disabled: busy || !form.inviteValid,
      }
    case "connect":
      if (s.connect.checking)
        return { key: "btn_wait", action: "none", disabled: true }
      // A refused certificate is on purpose: a new invite helps, retrying doesn't
      if (s.connect.problem === "cert_mismatch")
        return { key: "btn_new_invite", action: "new_invite", disabled: busy }
      return { key: "btn_retry", action: "retry", disabled: busy }
    case "register":
      return {
        key: busy ? "btn_wait" : "btn_register",
        action: "register",
        disabled:
          busy || !form.usernameValid || s.status?.registration === "NONE",
      }
    case "locate":
      if (s.revived) {
        return {
          key: s.revived.problem === "not_set" ? "btn_download" : "btn_check",
          action: "revived_install",
          disabled: busy,
        }
      }
      if (s.game.problem === "unverified")
        return { key: "btn_verify", action: "verify", disabled: busy }
      return { key: "btn_locate", action: "locate", disabled: busy }
    case "runtime":
      switch (s.runtime.state) {
        case "untrusted":
          return { key: "btn_trust", action: "trust", disabled: busy }
        case "missing":
          return {
            key: "btn_install",
            action: "install_runtime",
            disabled: busy,
          }
        case "outdated":
          return {
            key: "btn_update",
            action: "install_runtime",
            disabled: busy,
          }
        default:
          return { key: "btn_waiting_host", action: "none", disabled: true }
      }
    case "installing":
      return { key: "btn_cancel", action: "cancel", disabled: false }
    case "ready":
      return {
        key: busy ? "btn_wait" : "btn_play",
        action: "play",
        disabled: busy,
      }
    case "running":
      return { key: "btn_running", action: "none", disabled: true }
  }
}

export const STEP_KEYS: readonly Key[] = [
  "step_join",
  "step_register",
  "step_game",
  "step_runtime",
  "step_play",
]

export function stepIndex(phase: Phase): number {
  switch (phase) {
    case "loading":
    case "join":
    case "connect":
      return 0
    case "register":
      return 1
    case "locate":
      return 2
    case "runtime":
    case "installing":
      return 3
    case "ready":
    case "running":
      return 4
  }
}

export function hintKey(s: Snapshot): Key | null {
  switch (s.phase) {
    case "join":
      return "hint_join"
    case "connect":
      if (s.connect.checking) return "hint_connecting"
      return s.connect.problem === "cert_mismatch"
        ? "hint_cert"
        : "hint_connect"
    case "register":
      return "hint_register"
    case "locate":
      if (s.revived)
        return s.revived.problem === "not_set"
          ? "hint_revived_download"
          : "hint_revived_check"
      return s.game.problem === "unverified" ? "hint_verify" : "hint_locate"
    case "runtime":
      if (s.runtime.state === "unavailable") return "hint_runtime_none"
      return s.runtime.state === "untrusted" ? "hint_trust" : "hint_runtime"
    case "ready":
      return "hint_ready"
    case "running":
      return "hint_running"
    default:
      return null
  }
}

// Switching servers, logging out or choosing another folder waits for any task and for the game
export function locked(s: Snapshot): boolean {
  return s.busy || s.task !== null || s.running.game
}

export interface TaskView {
  key: Key
  percent: number
  bytes: string | null
  speed: string | null
  eta: string | null
}

export function taskView(t: TaskProgress, lang: Language): TaskView {
  const key: Key =
    t.kind === "runtime"
      ? "task_runtime"
      : t.kind === "download"
        ? "task_download"
        : t.kind === "dlls"
          ? "task_dlls"
          : t.kind === "install"
            ? "task_install"
            : "task_verify"
  const moving =
    t.kind === "runtime" || t.kind === "download" || t.kind === "install"
  return {
    key,
    percent: formatPercent(t.doneBytes, t.totalBytes),
    // The 1.4.4 quick check counts files, not bytes
    bytes:
      t.kind === "dlls" ||
      (t.kind === "verify" && t.totalBytes === t.filesTotal)
        ? null
        : `${formatBytes(t.doneBytes, lang)} / ${formatBytes(t.totalBytes, lang)}`,
    speed:
      moving && t.bytesPerSecond > 0
        ? formatSpeed(t.bytesPerSecond, lang)
        : null,
    eta: moving && t.etaSeconds !== null ? formatDuration(t.etaSeconds) : null,
  }
}

export function installingTitle(
  s: Snapshot,
  kind: TaskProgress["kind"]
): { title: Key; text: Key } {
  if (s.revived) {
    return {
      title:
        kind === "download"
          ? "revived_installing_download"
          : kind === "dlls"
            ? "revived_installing_dlls"
            : "revived_installing_verify",
      text: "revived_installing_text",
    }
  }
  return kind === "runtime"
    ? { title: "installing_runtime_title", text: "installing_runtime_text" }
    : { title: "installing_verify_title", text: "installing_verify_text" }
}

export function graphicsKey(g: GraphicsPreset): Key {
  return g < 0 ? "gfx_menu" : (`gfx_${g}` as Key)
}

export function whereKey(w: PlayerWhere): Key {
  return `where_${w}` as Key
}

export function kindKey(k: InstanceKind): Key {
  return `kind_${k}` as Key
}

export function sourceKey(source: GameSource): Key {
  return `game_source_${source}` as Key
}

export function runtimeStateKey(state: RuntimeState): Key {
  return `runtime_state_${state}` as Key
}

export function instanceTitle(i: StatusInstance): Message {
  switch (i.kind) {
    case "city":
      return { key: "inst_title_city" }
    case "dojo":
      return { key: "inst_title_dojo" }
    case "tutorial":
      return { key: "inst_title_tutorial" }
    case "hunt":
      return {
        key: "inst_title_hunt",
        vars: { behemoth: i.title || i.behemoth || { key: "kind_hunt" } },
      }
  }
}

// A SHA-256 in groups of four, and its first 16 digits: enough to compare with what the host shows
export function groupedHex(hex: string): string {
  return hex.match(/.{1,4}/g)?.join(" ") ?? hex
}

export function shortHex(hex: string): string {
  return `${groupedHex(hex.slice(0, 16))}…`
}

// Reforged's registration rule, as protocol::account::check_username reads it
export type UsernameCheck =
  "empty" | "too_short" | "too_long" | "bad_chars" | "ok"

export function checkUsername(input: unknown): UsernameCheck {
  if (typeof input !== "string" || input.length === 0) return "empty"
  if (!/^[A-Za-z0-9_]+$/.test(input)) return "bad_chars"
  if (input.length < 3) return "too_short"
  if (input.length > 16) return "too_long"
  return "ok"
}

// What an invite names, for the preview and the join question. Display only: the backend reads the
// invite itself (check_invite, submit_invite) and refuses anything that isn't exactly an invite.
export interface InviteSummary {
  name: string
  host: string
  port: number | null
  mode: ServerMode
  fp: string | null
}

export function inviteSummary(text: string): InviteSummary | null {
  const query = text.trim().split("?")[1]
  if (query === undefined) return null
  let params: URLSearchParams
  try {
    params = new URLSearchParams(query)
  } catch {
    return null
  }
  const get = (...names: string[]) =>
    names.map((n) => params.get(n)).find((v) => v !== null) ?? null
  const host = get("host", "h")
  const name = get("name", "n")
  if (host === null || name === null) return null
  const port = Number(get("port", "p"))
  const mode: ServerMode = get("mode") === "public" ? "public" : "private"
  return {
    name,
    host,
    port:
      Number.isInteger(port) && port > 0
        ? port
        : mode === "public"
          ? 443
          : null,
    mode,
    fp: mode === "public" ? get("fp") : null,
  }
}
