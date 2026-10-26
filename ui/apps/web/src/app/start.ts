// Astro writes @vitejs/plugin-react's Fast Refresh preamble only for client:* islands, which the
// launcher's CSP rules out (astro.config.mjs). Without it the dev server refuses every .tsx module.
async function start() {
  if (import.meta.env.DEV) {
    const refresh = "/@react-refresh"
    const { default: runtime } = await import(/* @vite-ignore */ refresh)
    runtime.injectIntoGlobalHook(window)
    Object.assign(window, {
      $RefreshReg$: () => {},
      $RefreshSig$: () => (type: unknown) => type,
      __vite_plugin_react_preamble_installed__: true,
    })
  }
  await import("@/app/mount")
}

void start()
