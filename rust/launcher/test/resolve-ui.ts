// Lets the tests load dainty-lass-ui's bridge (ui/packages/bridge/src) from its sources without installing
// the UI: its relative imports name no extension, as Vite reads them, and its @tauri-apps/api is this
// package's copy (the same 2.12.4), so the IPC mocks the tests install are the ones it calls.

import { registerHooks } from "node:module";

const here = import.meta.url;

registerHooks({
  resolve(specifier, context, next) {
    if (specifier === "@tauri-apps/api" || specifier.startsWith("@tauri-apps/api/")) {
      return next(specifier, { ...context, parentURL: here });
    }
    const fromUi = context.parentURL?.includes("/ui/packages/") === true;
    if (fromUi && specifier.startsWith(".") && !/\.[cm]?[jt]s$/.test(specifier)) {
      return next(`${specifier}.ts`, context);
    }
    return next(specifier, context);
  },
});
