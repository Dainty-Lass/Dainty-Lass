// The page's way into the launcher backend. src/generated/types.ts is written by DaintyLass from
// rust/launcher/src-tauri/src/bridge (RR_UPDATE_GENERATED=1) and stays AGPL-3.0-only with the bridge
// types it comes from (DD26); this file is MIT.
import { invoke, isTauri } from "@tauri-apps/api/core"
import { getCurrentWindow } from "@tauri-apps/api/window"

import type { CommandName } from "./generated/types"

export type * from "./generated/types"

// The isolation hook drops a call it refuses without settling it, so every call has a deadline.
export const CALL_TIMEOUT_MS = 60_000

export class BridgeError extends Error {
  readonly command: string

  constructor(command: string, reason: string) {
    super(`${command}: ${reason}`)
    this.name = "BridgeError"
    this.command = command
  }
}

export function inLauncher(): boolean {
  return isTauri()
}

// The window has no frame (the launcher's window.rs), so the page draws its own controls.
export function closeWindow(): Promise<void> {
  return getCurrentWindow().close()
}

export function call<T>(
  command: CommandName,
  args: Record<string, unknown> = {},
  timeoutMs = CALL_TIMEOUT_MS
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new BridgeError(command, "no answer")),
      timeoutMs
    )
    invoke<T>(command, args).then(
      (value) => {
        clearTimeout(timer)
        resolve(value)
      },
      (error: unknown) => {
        clearTimeout(timer)
        reject(new BridgeError(command, String(error)))
      }
    )
  })
}
