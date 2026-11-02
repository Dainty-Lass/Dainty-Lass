// What the Game screen shows for the Epic account, the version list and an install (DE7, DD22), as pure
// functions over the bridge's types, so the decisions are tested without a DOM (epic.test.ts). Every string is
// a catalog key.
import type {
  EpicLibraryCheck,
  EpicView,
  GameVersionEntry,
  InstallInfo,
  InstallState,
  LauncherError,
  TaskProgress,
} from "@workspace/bridge"
import type { Key } from "@workspace/i18n"

import type { Message } from "./model"

// The page Epic's sign-in opens, for the paste fallback. The same address as epic::auth::LOGIN_URL in
// DaintyLass's crates/epic; the sign-in window opens it itself, so only the fallback shows it.
export const EPIC_LOGIN_URL =
  "https://www.epicgames.com/id/login?redirectUrl=https%3A%2F%2Fwww.epicgames.com%2Fid%2Fapi%2Fredirect%3FclientId%3D34a02cf8f4414e29b15921876da36f9a%26responseType%3Dcode"

const RUNNING: readonly InstallState[] = [
  "preparing",
  "downloading",
  "paused",
  "finishing",
]

// An install the backend is carrying out now (a paused one still holds the slot)
export function isRunning(state: InstallState): boolean {
  return RUNNING.includes(state)
}

export type VersionAction =
  "install" | "continue" | "retry" | "pause" | "resume" | "cancel" | "use"

export interface VersionButton {
  action: VersionAction
  disabled: boolean
}

export interface VersionRow {
  // The state in words; `detail` is the install folder
  status: Message
  // Why the last attempt stopped
  error: LauncherError | null
  buttons: VersionButton[]
  // Why a button that would start an install is off
  hint: Key | null
  running: boolean
}

export interface RowContext {
  account: EpicView | null
  // Null until the library has been checked
  library: EpicLibraryCheck | null
  // Another version is being installed
  otherRunning: boolean
  // The launcher's game folder is this version's
  inUse: boolean
}

// Backslashes and a trailing slash don't tell two spellings of a folder apart; a Windows drive letter is
// not case-sensitive
export function sameFolder(a: string | null, b: string | null): boolean {
  if (a === null || b === null) return false
  const norm = (path: string) => {
    const slashes = path.replace(/\\/g, "/").replace(/\/+$/, "")
    return /^[A-Za-z]:/.test(slashes) ? slashes.toLowerCase() : slashes
  }
  return norm(a) === norm(b)
}

// The library check as the page knows it: true or false once Epic answered, null while it is unknown
export function owned(library: EpicLibraryCheck | null): boolean | null {
  return library?.ok ? library.owned : null
}

export function versionRow(
  entry: GameVersionEntry,
  info: InstallInfo | null,
  ctx: RowContext
): VersionRow {
  const none = (status: Message): VersionRow => ({
    status,
    error: null,
    buttons: [],
    hint: null,
    running: false,
  })
  // 1.4.4 and any other version with no Epic build: the player locates a folder they already have
  if (entry.install === "locate" || entry.epic === null) {
    return none({ key: "gm_ver_locate" })
  }
  const state: InstallState = info?.state ?? "not_installed"
  const folder = info?.folder ?? undefined
  const running = isRunning(state)
  if (state === "installed") {
    return {
      ...none({ key: "gm_ver_installed", detail: folder }),
      buttons: ctx.inUse ? [] : [{ action: "use", disabled: false }],
    }
  }
  if (running) {
    const key: Key =
      state === "preparing"
        ? "gm_state_preparing"
        : state === "downloading"
          ? "gm_state_downloading"
          : state === "paused"
            ? "gm_state_paused"
            : "gm_state_finishing"
    const buttons: VersionButton[] =
      state === "paused"
        ? [
            { action: "resume", disabled: false },
            { action: "cancel", disabled: false },
          ]
        : state === "finishing"
          ? []
          : [
              { action: "pause", disabled: false },
              { action: "cancel", disabled: false },
            ]
    return {
      status: { key, detail: folder },
      error: null,
      buttons,
      hint: null,
      running: true,
    }
  }
  // Not installed, stopped part-way, or failed: the ways to start are the same
  if (entry.epic.manifest !== "ready" && state !== "resumable") {
    return {
      ...none({
        key:
          entry.epic.manifest === "invalid"
            ? "gm_ver_manifest_invalid"
            : "gm_ver_manifest_unreachable",
      }),
      error: info?.error ?? null,
    }
  }
  const action: VersionAction =
    state === "resumable"
      ? "continue"
      : state === "failed"
        ? "retry"
        : "install"
  let hint: Key | null = null
  if (!ctx.account?.signedIn) hint = "gm_sign_in_first"
  else if (owned(ctx.library) === false) hint = "gm_not_owned_short"
  else if (ctx.otherRunning) hint = "gm_other_running"
  return {
    status: {
      key:
        state === "resumable"
          ? "gm_ver_resumable"
          : state === "failed"
            ? "gm_ver_failed"
            : "gm_ver_not_installed",
    },
    error: info?.error ?? null,
    buttons: [{ action, disabled: hint !== null }],
    hint,
    running: false,
  }
}

export const ACTION_KEYS: Record<VersionAction, Key> = {
  install: "gm_install",
  continue: "gm_continue",
  retry: "gm_retry",
  pause: "gm_pause",
  resume: "gm_resume",
  cancel: "gm_stop",
  use: "gm_use",
}

// The numbers to draw for a running install: the channel's progress is newer than the polled one, but it
// carries no version, so it counts only if it is the same download (same total) as the one polled for
export function installProgress(
  info: InstallInfo | null,
  channel: TaskProgress | null
): TaskProgress | null {
  if (!info || !isRunning(info.state)) return null
  if (
    channel &&
    channel.kind === "install" &&
    (info.progress === null || channel.totalBytes === info.progress.totalBytes)
  )
    return channel
  return info.progress
}

// Download and on-disk sizes for the list, once the manifest has given them
export function versionSizes(
  entry: GameVersionEntry
): { download: number; install: number } | null {
  const epic = entry.epic
  if (!epic || epic.downloadBytes === null || epic.installBytes === null)
    return null
  return { download: epic.downloadBytes, install: epic.installBytes }
}
