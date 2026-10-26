// One call into the launcher backend. The isolation hook drops a call it refuses without settling it, so
// every call has a deadline.
import { invoke, isTauri } from "@tauri-apps/api/core"

import type { CommandName } from "./generated/types"

// A call that waits on the player (a file dialog) or on a long task (the executable check, the runtime
// download) gets an hour; any other a minute
export const CALL_TIMEOUT_MS = 60_000
export const LONG_TIMEOUT_MS = 3_600_000

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
