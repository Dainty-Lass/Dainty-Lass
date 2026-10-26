// @ts-check

import tailwindcss from "@tailwindcss/vite"
import { defineConfig } from "astro/config"
import react from "@astrojs/react"

// The launcher serves dist/ under a CSP of script-src 'self' and style-src 'self' (DaintyLass's
// tauri.conf.json). So: no inline stylesheet or script in the built HTML, and no client:* islands, whose
// loader Astro writes inline; React mounts from a bundled script instead (src/app/start.ts).
// https://astro.build/config
export default defineConfig({
  output: "static",
  build: {
    inlineStylesheets: "never",
  },
  devToolbar: {
    enabled: false,
  },
  // tauri.conf.json's devUrl
  server: {
    port: 4321,
  },
  vite: {
    plugins: [tailwindcss()],
    build: {
      assetsInlineLimit: 0,
    },
  },
  integrations: [react()],
})
