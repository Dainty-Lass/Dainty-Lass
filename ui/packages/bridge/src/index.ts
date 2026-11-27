import { mockLauncher } from "./mock";
import type { LauncherApi } from "./types";

export type * from "./generated/types";
export { mockLauncher } from "./mock";
export type { LauncherApi, LegacyClaimAnswer } from "./types";

let cached: LauncherApi | undefined;

/**
 * The bridge inside Tauri, the stand-in anywhere else (`bun run dev` in a browser). The real one is
 * imported lazily so a browser build never evaluates @tauri-apps/api's invoke.
 */
export async function getLauncher(): Promise<LauncherApi> {
  if (cached) return cached;
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
    const { api } = await import("./api");
    // No browser context menu, as in the Electron launcher (debug builds still open devtools with F12).
    document.addEventListener("contextmenu", (event) => event.preventDefault());
    cached = api;
  } else {
    cached = mockLauncher();
  }
  return cached;
}

export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
