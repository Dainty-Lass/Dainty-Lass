// Gives the page `window.launcher`: page/api.ts over Tauri's IPC. index.html loads this module before its
// own script, which reads `window.launcher` once. dainty-lass-ui (DU1) imports the bridge itself, and this
// file goes with it.

import { api } from "./api.ts";

// Tauri sets `__TAURI_INTERNALS__` before any page script runs
if ("__TAURI_INTERNALS__" in window) {
  Object.defineProperty(window, "launcher", { value: Object.freeze(api), writable: false, configurable: false });
  // No browser context menu, as in the Electron launcher (debug builds still open devtools with F12)
  document.addEventListener("contextmenu", (event) => event.preventDefault());
}
